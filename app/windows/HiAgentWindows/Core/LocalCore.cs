using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Runtime.InteropServices;

namespace HiAgent.Windows.Core;

/// <summary>
/// The engine, as a child process.
///
/// This is the part of the Windows app that has no equivalent on the phones:
/// "host and client are capabilities of an app instance, never properties of a
/// platform" (`docs/arch/topology.md`), and a desktop answers yes to hosting.
/// So the shell starts `hi-agent.exe`, keeps it running, and — through the job
/// object below — makes sure it never outlives the shell.
///
/// Supervision, not management. The engine owns its data directory, its
/// runtime provisioning and its own restarts of anything below it; all this
/// does is start it, watch for it going away, and start it again.
/// </summary>
internal sealed class LocalCore : IDisposable
{
    /// <summary>The port a desktop install is reached on, per `src/main.rs`.</summary>
    internal const int PreferredPort = 12358;

    private readonly SemaphoreSlim _gate = new(1, 1);
    private readonly CancellationTokenSource _stopping = new();

    private Process? _process;
    private SafeJobHandle? _job;
    private Task? _supervisor;

    /// <summary>Where the engine is, once it answers. Null until then.</summary>
    internal Uri? BaseUrl { get; private set; }

    /// <summary>
    /// True when this shell did not start the engine because one was already
    /// answering on the preferred port — a developer running `cargo run`, or a
    /// shell that crashed and left its child behind. Adopting is right: two
    /// engines over one data directory is the failure worth avoiding, and it is
    /// worse than not being the one who started it.
    /// </summary>
    internal bool Adopted { get; private set; }

    /// <summary>Last thing that went wrong, for the stage to show.</summary>
    internal string? Failure { get; private set; }

    /// <summary>Raised when the engine's reachability changes.</summary>
    internal event Action? Changed;

    /// <summary>
    /// Start the engine, or adopt one already running, and keep it up until
    /// disposed. Returns as soon as an address is known — health is polled by
    /// the caller, because a core that is up and one that answers are different
    /// facts and the second is the one the face needs.
    /// </summary>
    internal async Task<Uri?> StartAsync()
    {
        await _gate.WaitAsync().ConfigureAwait(false);
        try
        {
            if (BaseUrl is not null)
            {
                return BaseUrl;
            }

            var existing = new Uri($"http://127.0.0.1:{PreferredPort}");
            if (await CoreClient.HealthAsync(existing).ConfigureAwait(false) is HealthState.Here)
            {
                Adopted = true;
                BaseUrl = existing;
                Log.Write($"adopted an engine already answering on {PreferredPort}");
                Changed?.Invoke();
                return BaseUrl;
            }

            var exe = AppPaths.EngineExe();
            if (exe is null)
            {
                Failure = "hi-agent.exe is not next to the app. Reinstall, or add a core that runs elsewhere.";
                Log.Write(Failure);
                Changed?.Invoke();
                return null;
            }

            var port = IsPortFree(PreferredPort) ? PreferredPort : FreePort();
            BaseUrl = new Uri($"http://127.0.0.1:{port}");
            _supervisor = Task.Run(() => SuperviseAsync(exe, port, _stopping.Token));
            return BaseUrl;
        }
        finally
        {
            _gate.Release();
        }
    }

    /// <summary>
    /// Start, watch, restart. The backoff exists for the case where the engine
    /// exits immediately and forever — a corrupt data directory, a missing
    /// dependency — where restarting in a tight loop would burn the machine and
    /// bury the reason in a log nobody can read.
    /// </summary>
    private async Task SuperviseAsync(string exe, int port, CancellationToken token)
    {
        var backoff = TimeSpan.FromSeconds(1);
        while (!token.IsCancellationRequested)
        {
            var startedAt = DateTimeOffset.UtcNow;
            try
            {
                var process = Spawn(exe, port);
                _process = process;
                Failure = null;
                Changed?.Invoke();
                await process.WaitForExitAsync(token).ConfigureAwait(false);

                if (token.IsCancellationRequested)
                {
                    return;
                }
                Log.Write($"engine exited with {process.ExitCode}");
                Failure = $"The agent stopped (exit {process.ExitCode}). Restarting.";
            }
            catch (OperationCanceledException)
            {
                return;
            }
            catch (Exception e)
            {
                Log.Write($"engine could not be started: {e}");
                Failure = $"The agent could not be started: {e.Message}";
            }

            Changed?.Invoke();

            // A run that lasted a while was working; the next failure is a new
            // one and deserves a fresh short wait rather than the last run's
            // accumulated punishment.
            backoff = DateTimeOffset.UtcNow - startedAt > TimeSpan.FromMinutes(1)
                ? TimeSpan.FromSeconds(1)
                : TimeSpan.FromSeconds(Math.Min(30, backoff.TotalSeconds * 2));

            try
            {
                await Task.Delay(backoff, token).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return;
            }
        }
    }

    private Process Spawn(string exe, int port)
    {
        var info = new ProcessStartInfo(exe)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            // The engine resolves relative paths — including its `./data`
            // fallback — against this. Its own directory is the least
            // surprising answer, and `--data-dir` means the fallback is never
            // reached anyway.
            WorkingDirectory = Path.GetDirectoryName(exe) ?? AppContext.BaseDirectory,
        };
        info.ArgumentList.Add("--port");
        info.ArgumentList.Add(port.ToString());
        info.ArgumentList.Add("--data-dir");
        info.ArgumentList.Add(AppPaths.EngineData);

        var process = new Process { StartInfo = info, EnableRaisingEvents = true };
        process.OutputDataReceived += (_, e) => AppendEngineLog(e.Data);
        process.ErrorDataReceived += (_, e) => AppendEngineLog(e.Data);

        Log.Write($"starting {exe} --port {port} --data-dir {AppPaths.EngineData}");
        process.Start();
        process.BeginOutputReadLine();
        process.BeginErrorReadLine();

        // Assign to the job *after* start: there is a window here in which a
        // crash of the shell leaves the engine running, and closing it properly
        // needs `CreateProcess` with a suspended thread. The window is
        // milliseconds, and the cost of a missed one is a stray engine that the
        // adoption path above then picks back up.
        AssignToJob(process);
        return process;
    }

    private static void AppendEngineLog(string? line)
    {
        if (string.IsNullOrEmpty(line))
        {
            return;
        }
        try
        {
            File.AppendAllText(AppPaths.EngineLog, line + Environment.NewLine);
        }
        catch
        {
            // Losing a log line is not worth taking down the reader.
        }
    }

    /// <summary>
    /// A job object with kill-on-close, so the engine cannot outlive the shell.
    ///
    /// Windows has no process groups and no orphan reaping: a child whose parent
    /// dies is simply re-parented and keeps running. For a process holding the
    /// agent's data directory that is the worst shape of failure — invisible,
    /// still writing, and in the way of the next start. The job is the OS
    /// mechanism for "these processes belong to that one", and it holds through
    /// a crash, a kill, and Task Manager's End Task.
    /// </summary>
    private void AssignToJob(Process process)
    {
        try
        {
            if (_job is null)
            {
                var handle = CreateJobObjectW(IntPtr.Zero, null);
                if (handle == IntPtr.Zero)
                {
                    throw new InvalidOperationException($"CreateJobObject failed ({Marshal.GetLastWin32Error()})");
                }
                _job = new SafeJobHandle(handle);

                var limits = new JobObjectExtendedLimitInformation
                {
                    BasicLimitInformation = new JobObjectBasicLimitInformation
                    {
                        LimitFlags = JobObjectLimitKillOnJobClose,
                    },
                };
                var size = Marshal.SizeOf<JobObjectExtendedLimitInformation>();
                var buffer = Marshal.AllocHGlobal(size);
                try
                {
                    Marshal.StructureToPtr(limits, buffer, false);
                    if (!SetInformationJobObject(handle, JobObjectExtendedLimitInformationClass, buffer, (uint)size))
                    {
                        throw new InvalidOperationException(
                            $"SetInformationJobObject failed ({Marshal.GetLastWin32Error()})");
                    }
                }
                finally
                {
                    Marshal.FreeHGlobal(buffer);
                }
            }

            if (!AssignProcessToJobObject(_job.DangerousGetHandle(), process.Handle))
            {
                throw new InvalidOperationException(
                    $"AssignProcessToJobObject failed ({Marshal.GetLastWin32Error()})");
            }
        }
        catch (Exception e)
        {
            // Not fatal: the engine runs, and `Dispose` still kills it on an
            // ordinary quit. What is lost is the guarantee on the disorderly
            // paths, which is worth a line in the log and not a refusal to run.
            Log.Write($"job object not applied, engine may outlive a crash: {e.Message}");
        }
    }

    private static bool IsPortFree(int port)
    {
        try
        {
            using var listener = new Socket(AddressFamily.InterNetwork, SocketType.Stream, ProtocolType.Tcp);
            listener.Bind(new IPEndPoint(IPAddress.Loopback, port));
            return true;
        }
        catch (SocketException)
        {
            return false;
        }
    }

    /// <summary>
    /// A port the OS says is free. Racy by nature — something can take it
    /// between the close and the engine's bind — and the supervisor's restart
    /// is what covers that, rather than a lock that cannot exist.
    /// </summary>
    private static int FreePort()
    {
        using var listener = new Socket(AddressFamily.InterNetwork, SocketType.Stream, ProtocolType.Tcp);
        listener.Bind(new IPEndPoint(IPAddress.Loopback, 0));
        return ((IPEndPoint)listener.LocalEndPoint!).Port;
    }

    public void Dispose()
    {
        _stopping.Cancel();
        try
        {
            if (_process is { HasExited: false })
            {
                // No console is attached, so there is no Ctrl+C to send and no
                // Windows equivalent of SIGTERM for a windowless process. The
                // engine is built to be killed — its state is on disk and its
                // session directory is written as it goes, which is what makes
                // resuming after a restart work at all.
                _process.Kill(entireProcessTree: true);
            }
        }
        catch (Exception e)
        {
            Log.Write($"engine kill: {e.Message}");
        }
        _process?.Dispose();
        // Closing the job kills anything still in it — the runtime's node
        // processes included.
        _job?.Dispose();
        _stopping.Dispose();
        _gate.Dispose();
    }

    private sealed class SafeJobHandle(IntPtr handle) : IDisposable
    {
        private IntPtr _handle = handle;

        internal IntPtr DangerousGetHandle() => _handle;

        public void Dispose()
        {
            if (_handle != IntPtr.Zero)
            {
                CloseHandle(_handle);
                _handle = IntPtr.Zero;
            }
        }
    }

    private const int JobObjectExtendedLimitInformationClass = 9;
    private const uint JobObjectLimitKillOnJobClose = 0x2000;

    [StructLayout(LayoutKind.Sequential)]
    private struct JobObjectBasicLimitInformation
    {
        public long PerProcessUserTimeLimit;
        public long PerJobUserTimeLimit;
        public uint LimitFlags;
        public UIntPtr MinimumWorkingSetSize;
        public UIntPtr MaximumWorkingSetSize;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint PriorityClass;
        public uint SchedulingClass;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct IoCounters
    {
        public ulong ReadOperationCount;
        public ulong WriteOperationCount;
        public ulong OtherOperationCount;
        public ulong ReadTransferCount;
        public ulong WriteTransferCount;
        public ulong OtherTransferCount;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct JobObjectExtendedLimitInformation
    {
        public JobObjectBasicLimitInformation BasicLimitInformation;
        public IoCounters IoInfo;
        public UIntPtr ProcessMemoryLimit;
        public UIntPtr JobMemoryLimit;
        public UIntPtr PeakProcessMemoryUsed;
        public UIntPtr PeakJobMemoryUsed;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr CreateJobObjectW(IntPtr attributes, string? name);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetInformationJobObject(
        IntPtr job, int infoClass, IntPtr info, uint length);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CloseHandle(IntPtr handle);
}
