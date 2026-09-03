using System.Diagnostics;
using HiAgent.Windows.Core;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;

namespace HiAgent.Windows;

/// <summary>
/// The process. On Windows the shell owns it — `main`, the message loop, the
/// tray, and everything that touches the desktop session — and the engine is a
/// child process it starts and supervises. That is the arrangement
/// `docs/arch/topology.md` describes for an app, and the one macOS is still
/// migrating toward from the other direction.
/// </summary>
public partial class App : Application
{
    /// <summary>
    /// Single-instance key. Two shells would mean two engines on two ports
    /// writing one data directory, which is the "one body per person" rule
    /// broken by accident.
    /// </summary>
    private const string InstanceKey = "dev.human-interface.hi-agent.windows";

    public static new App Current => (App)Application.Current;

    internal AppModel Model { get; } = new();

    private MainWindow? _window;

    public App()
    {
        InitializeComponent();
        UnhandledException += (_, e) =>
        {
            // A crash with no console and no log is a support ticket with no
            // content. The shell's own log sits beside the engine's.
            Log.Write($"unhandled: {e.Exception}");
        };
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        var instance = AppInstance.FindOrRegisterForKey(InstanceKey);
        if (!instance.IsCurrent)
        {
            // Hand the activation to the shell already running, then leave.
            // `Exit()` from inside `OnLaunched` is not reliable before the
            // window exists; killing the redundant process is what the App SDK
            // sample does and what the situation actually calls for.
            var current = AppInstance.GetCurrent().GetActivatedEventArgs();
            instance.RedirectActivationToAsync(current).AsTask().GetAwaiter().GetResult();
            Process.GetCurrentProcess().Kill();
            return;
        }

        instance.Activated += (_, _) =>
        {
            // A second launch — the Start Menu shortcut, the desktop icon —
            // means "show me the agent", not "start another one".
            _window?.DispatcherQueue.TryEnqueue(() => _window?.Reveal());
        };

        // The window is constructed but not activated. The tray is the app's
        // presence, so a launch is quiet: the icon appears, the engine starts,
        // and the face opens when the person asks for it. `TaskbarIcon` is
        // created in code and force-created for exactly this reason — an icon
        // declared in an unshown window's XAML would never exist.
        _window = new MainWindow(Model);

        _ = Model.StartAsync();
    }

    /// <summary>
    /// Shut the engine down before the process goes. Ordinary quit path; the
    /// job object in <see cref="LocalCore"/> is the backstop for the paths that
    /// are not ordinary.
    /// </summary>
    internal void Quit()
    {
        // The tray icon first: a notification-area icon whose process is gone
        // stays drawn until something hovers it.
        _window?.Shutdown();
        Model.Dispose();
        Exit();
    }
}
