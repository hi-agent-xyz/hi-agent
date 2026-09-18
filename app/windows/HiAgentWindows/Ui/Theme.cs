using HiAgent.Windows.Core;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;

namespace HiAgent.Windows.Ui;

/// <summary>
/// Light or dark, for the shell's own chrome.
///
/// **The core persists the choice and the shell applies it** — the rule
/// `docs/core-shell-config-api.md` states, and the reason the Settings window
/// writes the value over HTTP and then calls in here rather than remembering it
/// locally. macOS does the same thing in two lines of `HiSettings.swift`
/// (`NSApp.appearance`); on Windows each window's root element carries the theme,
/// so there is a list of them to tell.
///
/// It matters beyond taste. The face paints `--bg-1` right under the title bar
/// and `MainWindow.ApplyTitleBarTheme` matches the bar to it — off the system
/// theme, which is not what the person chose. A stored "dark" against a light
/// Windows would draw a pale strip across the top of a dark page, which is
/// exactly the seam that code exists to avoid.
/// </summary>
internal static class Theme
{
    private static DispatcherQueue? _ui;

    /// <summary>What every window's root element is set to. Read on the UI thread.</summary>
    internal static ElementTheme Current { get; private set; } = ElementTheme.Default;

    private static event Action? Changed;

    /// <summary>
    /// Name the thread the windows live on, once, from the first window. Applying
    /// a theme touches XAML, and the value arrives from an HTTP call that does not.
    /// </summary>
    internal static void Install(DispatcherQueue ui) => _ui ??= ui;

    /// <summary>
    /// Take the engine's word for it — `"system"`, `"light"` or `"dark"`, as
    /// `GET /api/settings` spells them. Anything else is the system's choice,
    /// which is also what an older engine that has never heard of this setting
    /// leaves behind.
    /// </summary>
    internal static void Apply(string? value)
    {
        if (_ui is { } ui && !ui.HasThreadAccess)
        {
            ui.TryEnqueue(() => Apply(value));
            return;
        }

        var next = value?.Trim().ToLowerInvariant() switch
        {
            "light" => ElementTheme.Light,
            "dark" => ElementTheme.Dark,
            _ => ElementTheme.Default,
        };
        if (next == Current)
        {
            return;
        }
        Current = next;
        Changed?.Invoke();
    }

    /// <summary>
    /// Put a window's root under the current theme and keep it there. Dispose to
    /// stop — a window that has closed still has a live handler otherwise, and
    /// the Settings window opens and closes as often as anyone likes.
    /// </summary>
    internal static IDisposable Bind(FrameworkElement root)
    {
        root.RequestedTheme = Current;
        void Handler() => root.RequestedTheme = Current;
        Changed += Handler;
        return new Subscription(() => Changed -= Handler);
    }

    /// <summary>
    /// Read the stored theme once the engine can answer, and apply it.
    ///
    /// No timeout, deliberately, and the same reason `AppModel.WaitForHealthAsync`
    /// has none: a first run provisions its whole runtime before it answers
    /// anything, which is minutes. Until then the shell shows the system's theme,
    /// which is the right thing to show while nobody has said otherwise.
    /// </summary>
    internal static async Task FollowEngineAsync(Uri core, CancellationToken token)
    {
        while (!token.IsCancellationRequested)
        {
            if (await CoreClient.HealthAsync(core, token).ConfigureAwait(false) is HealthState.Here)
            {
                try
                {
                    var snapshot = await new SettingsClient(core).GetAsync(token).ConfigureAwait(false);
                    Apply(snapshot.Appearance.Theme.Value);
                }
                catch (Exception e) when (e is not OperationCanceledException)
                {
                    // The system theme is a working answer, so this is a log line
                    // rather than something to tell the person about.
                    Log.Write($"theme: {e.Message}");
                }
                return;
            }

            try
            {
                await Task.Delay(TimeSpan.FromSeconds(2), token).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return;
            }
        }
    }

    private sealed class Subscription(Action release) : IDisposable
    {
        public void Dispose() => release();
    }
}
