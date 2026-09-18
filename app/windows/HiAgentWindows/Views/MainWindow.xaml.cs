using HiAgent.Windows.Core;
using HiAgent.Windows.Ui;
using HiAgent.Windows.Views;
using Microsoft.UI;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;

namespace HiAgent.Windows;

/// <summary>
/// The face's frame, and the app's only window. It shows the core's page or a
/// sentence about why it cannot yet — never both.
/// </summary>
public sealed partial class MainWindow : Window
{
    private readonly AppModel _model;
    private readonly CoreWebView _face;
    private readonly TrayIcon _tray;
    private readonly IDisposable _theme;
    private readonly CancellationTokenSource _stopping = new();

    /// <summary>The one Settings window, while it is open. A second "Settings…" raises the first.</summary>
    private SettingsWindow? _settings;

    /// <summary>Whether the engine has been asked what theme it holds. Once, per run.</summary>
    private bool _themeAsked;

    internal MainWindow(AppModel model)
    {
        InitializeComponent();
        _model = model;

        Title = "Hi Agent";
        // The windows live on this thread, and a theme arrives from an HTTP call
        // that does not.
        Theme.Install(DispatcherQueue);
        _theme = Theme.Bind(Root);
        // `global::` throughout: this app's own namespace is `HiAgent.Windows`,
        // so a bare `Windows.Graphics` binds to it and not to the platform.
        AppWindow.Resize(new global::Windows.Graphics.SizeInt32(1100, 760));
        TrySetIcon();
        ApplyTitleBarTheme();
        Root.ActualThemeChanged += (_, _) => ApplyTitleBarTheme();

        // Closing the window puts the agent away; it does not stop it. The
        // tray is the app's presence, and an agent that has to be running to
        // hear you should not be quit by the same gesture that tidies a window.
        AppWindow.Closing += (_, args) =>
        {
            args.Cancel = true;
            AppWindow.Hide();
        };

        _face = new CoreWebView(Face, model);
        _tray = new TrayIcon(model, this);

        model.StateChanged += OnStateChanged;
        Render();
    }

    /// <summary>Show the window and put it in front. The tray and a second launch both land here.</summary>
    internal void Reveal()
    {
        AppWindow.Show();
        AppWindow.MoveInZOrderAtTop();
        Activate();
    }

    internal void Hide() => AppWindow.Hide();

    /// <summary>Give the tray icon back before the process ends. See <see cref="App.Quit"/>.</summary>
    internal void Shutdown()
    {
        _stopping.Cancel();
        _model.StateChanged -= OnStateChanged;
        _theme.Dispose();
        _tray.Dispose();
    }

    private void OnStateChanged()
    {
        // The model changes state on whatever thread noticed; the window is
        // only ever touched on its own.
        DispatcherQueue.TryEnqueue(Render);
    }

    private void Render()
    {
        var stage = _model.Stage;
        var ready = stage is CoreStage.Ready;

        Face.Visibility = ready ? Visibility.Visible : Visibility.Collapsed;
        Stage.Visibility = ready ? Visibility.Collapsed : Visibility.Visible;
        Spinner.IsActive = stage is CoreStage.Connecting or CoreStage.Waiting;
        RetryButton.Visibility = stage is CoreStage.Failed or CoreStage.Waiting
            ? Visibility.Visible
            : Visibility.Collapsed;
        AddCoreButton.Visibility = stage is CoreStage.Empty or CoreStage.Failed
            ? Visibility.Visible
            : Visibility.Collapsed;

        StageTitle.Text = stage switch
        {
            CoreStage.Empty => "No agent yet",
            CoreStage.Connecting => "Starting the agent…",
            CoreStage.Waiting => "Waiting for the agent",
            CoreStage.Failed => "The agent could not be reached",
            _ => string.Empty,
        };

        StageDetail.Text = _model.StageDetail ?? string.Empty;
        StageDetail.Visibility = string.IsNullOrEmpty(_model.StageDetail)
            ? Visibility.Collapsed
            : Visibility.Visible;

        // The engine holds the chosen theme, and this is the first moment there
        // is an address to ask at. Waiting for it to answer is the follower's
        // own job, so this only ever starts once.
        if (!_themeAsked && _model.LocalCoreUrl is { } local)
        {
            _themeAsked = true;
            _ = Theme.FollowEngineAsync(local, _stopping.Token);
        }

        _face.Sync();
        _tray.Sync();
    }

    private async void OnRetry(object sender, RoutedEventArgs e)
    {
        if (_model.Attached is { } entry)
        {
            await _model.AttachAsync(entry.Id);
        }
    }

    private void OnAddCore(object sender, RoutedEventArgs e) => ShowPairWindow();

    internal void ShowPairWindow()
    {
        var window = new AddAgentWindow(_model);
        window.Activate();
    }

    /// <summary>
    /// Open — or raise — Settings. A preferences window is a singleton on every
    /// desktop, and a second "Settings…" brings the first one forward.
    ///
    /// It configures **the agent on this computer**, which is the only one it
    /// can: the engine's config surface is loopback-gated, and a remote agent's
    /// credentials and reachability are its own machine's business. When this
    /// install hosts no engine there is nothing to open, and the tray says so by
    /// greying the item rather than by opening an empty window.
    /// </summary>
    internal void ShowSettingsWindow()
    {
        if (_settings is { } open)
        {
            open.Activate();
            return;
        }
        if (_model.LocalCoreUrl is not { } local)
        {
            return;
        }

        var window = new SettingsWindow(local);
        window.Closed += (_, _) => _settings = null;
        _settings = window;
        window.Activate();
    }

    private void TrySetIcon()
    {
        try
        {
            AppWindow.SetIcon(Path.Combine(AppContext.BaseDirectory, "Assets", "HiAgent.ico"));
        }
        catch (Exception e)
        {
            Log.Write($"window icon: {e.Message}");
        }
    }

    /// <summary>
    /// Keep the title bar the same colour as the page under it.
    ///
    /// The face paints `--bg-1` across the strip directly below the bar
    /// (`src/appearance/web/src/ui/global.css`), so a default-coloured bar draws
    /// a seam across the top of the window in exactly the place a person reads
    /// as the app's edge. macOS solves this in `apply_face_theme`; this is the
    /// same fix, and the two have to be changed together when the token moves.
    /// </summary>
    private void ApplyTitleBarTheme()
    {
        if (!AppWindowTitleBar.IsCustomizationSupported())
        {
            // Windows 10 before 1809-era builds: the bar is the system's and
            // there is nothing to match it with. The face still renders.
            return;
        }

        var dark = Root.ActualTheme == ElementTheme.Dark;
        var background = dark
            ? ColorHelper.FromArgb(255, 0x2B, 0x27, 0x20)   // --bg-1, dark
            : ColorHelper.FromArgb(255, 0xFF, 0xFF, 0xFF);  // --bg-1, light
        var foreground = dark
            ? ColorHelper.FromArgb(255, 0xEA, 0xE6, 0xDE)
            : ColorHelper.FromArgb(255, 0x1C, 0x1B, 0x18);

        var bar = AppWindow.TitleBar;
        bar.BackgroundColor = background;
        bar.InactiveBackgroundColor = background;
        bar.ForegroundColor = foreground;
        bar.InactiveForegroundColor = foreground;
        bar.ButtonBackgroundColor = background;
        bar.ButtonInactiveBackgroundColor = background;
        bar.ButtonForegroundColor = foreground;
        bar.ButtonInactiveForegroundColor = foreground;
    }
}
