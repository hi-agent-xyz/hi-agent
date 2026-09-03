namespace HiAgent.Windows.Core;

/// <summary>
/// Every directory this app reads or writes, in one place, because two of them
/// have to agree with something outside this project.
/// </summary>
internal static class AppPaths
{
    /// <summary>
    /// The shell's own state: the roster, the log, the WebView2 profile. Local
    /// rather than roaming — none of it is worth carrying to another machine,
    /// and the WebView2 profile is a cache that must never be on a roaming
    /// share.
    /// </summary>
    internal static string ShellData { get; } = EnsureDir(Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        "Hi Agent"));

    /// <summary>
    /// The engine's data directory — the whole agent, per `docs/data-dir-layout.md`.
    ///
    /// This value must match `directories::ProjectDirs::from("dev",
    /// "human-interface", "hi-agent").data_dir()`, which on Windows is
    /// `%APPDATA%\human-interface\hi-agent\data`, because that is what the
    /// engine picks for itself when it knows it is installed.
    ///
    /// It has to be passed explicitly all the same. `default_data_dir` in
    /// `src/main.rs` only reaches for the OS data directory when
    /// `bundle::resources_dir()` says it is inside a macOS `.app`; everywhere
    /// else it falls back to `./data`, relative to the working directory. An
    /// installed Windows engine launched from `%LOCALAPPDATA%\Programs\Hi Agent`
    /// would therefore put the person's memory inside the program directory,
    /// where the uninstaller's promise to leave user data alone stops being
    /// true. `--data-dir` is one flag and it settles that.
    ///
    /// Roaming, not local, deliberately: this is the agent, not a cache.
    /// </summary>
    internal static string EngineData { get; } = EnsureDir(Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "human-interface", "hi-agent", "data"));

    /// <summary>WebView2's profile. Cookies live here, and are cleared on detach.</summary>
    internal static string WebViewData { get; } = EnsureDir(Path.Combine(ShellData, "webview"));

    internal static string RosterFile { get; } = Path.Combine(ShellData, "roster.json");

    internal static string ShellLog { get; } = Path.Combine(ShellData, "shell.log");

    internal static string EngineLog { get; } = Path.Combine(ShellData, "engine.log");

    /// <summary>
    /// The engine, beside the shell. The installer puts `hi-agent.exe` and
    /// `HiAgent.exe` in one directory, so this resolves without configuration —
    /// and returns null in a dev checkout where only the shell was built, which
    /// is a state the app shows rather than crashes on.
    /// </summary>
    internal static string? EngineExe()
    {
        var dir = AppContext.BaseDirectory;
        var exe = Path.Combine(dir, "hi-agent.exe");
        return File.Exists(exe) ? exe : null;
    }

    private static string EnsureDir(string path)
    {
        try
        {
            Directory.CreateDirectory(path);
        }
        catch (Exception e)
        {
            Log.Write($"could not create {path}: {e.Message}");
        }
        return path;
    }
}

/// <summary>
/// A log file, because a windowed process has no console and a person who is
/// told "it did not start" needs somewhere to look. Appends, never rotates on
/// its own; the engine writes far more than this does.
/// </summary>
internal static class Log
{
    private static readonly object Gate = new();

    internal static void Write(string message)
    {
        var line = $"{DateTimeOffset.Now:yyyy-MM-dd HH:mm:ss.fff} {message}{Environment.NewLine}";
        try
        {
            lock (Gate)
            {
                File.AppendAllText(AppPaths.ShellLog, line);
            }
        }
        catch
        {
            // A log that throws would take the app with it for nothing.
        }
        System.Diagnostics.Debug.Write(line);
    }
}
