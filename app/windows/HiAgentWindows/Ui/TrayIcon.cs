using System.Diagnostics;
using System.Windows.Input;
using H.NotifyIcon;
using HiAgent.Windows.Core;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;

namespace HiAgent.Windows.Ui;

/// <summary>
/// The notification-area icon, which is the app's real presence on Windows —
/// the window comes and goes, this does not. The macOS twin is the menu-bar
/// tray in `macos_tray.rs`, and the menu is deliberately the same short list.
///
/// Built in code rather than declared in XAML because the window it would be
/// declared in is not shown at launch, and an icon in an unshown window's tree
/// is never created. <c>ForceCreate</c> is the supported way to say that.
/// </summary>
internal sealed class TrayIcon : IDisposable
{
    private readonly AppModel _model;
    private readonly MainWindow _window;
    private readonly TaskbarIcon _icon;
    private readonly MenuFlyout _menu = new();

    internal TrayIcon(AppModel model, MainWindow window)
    {
        _model = model;
        _window = window;

        _icon = new TaskbarIcon
        {
            ToolTipText = "Hi Agent",
            ContextFlyout = _menu,
            // Left click opens the face; the menu is the right-click.
            LeftClickCommand = new Action<object?>(_ => _window.Reveal()).AsCommand(),
            NoLeftClickDelay = true,
        };

        try
        {
            _icon.IconSource = new BitmapImage(new Uri("ms-appx:///Assets/HiAgent.ico"));
        }
        catch (Exception e)
        {
            // No icon is survivable — an unnamed blank in the tray still opens
            // the menu. Failing to start over it would not be.
            Log.Write($"tray icon image: {e.Message}");
        }

        Rebuild();
        _icon.ForceCreate();
    }

    /// <summary>Re-read the model. Cheap, and the roster is a handful of entries.</summary>
    internal void Sync() => Rebuild();

    private void Rebuild()
    {
        _menu.Items.Clear();

        _menu.Items.Add(Item("Open Hi Agent", () => _window.Reveal()));
        _menu.Items.Add(new MenuFlyoutSeparator());

        var roster = _model.Roster;
        if (roster.Count > 1)
        {
            // Only worth showing when there is a choice to make. One core is
            // not a list, it is the agent.
            var attached = _model.Attached?.Id;
            foreach (var entry in roster)
            {
                var item = new ToggleMenuFlyoutItem
                {
                    Text = entry.Label,
                    IsChecked = entry.Id == attached,
                };
                var id = entry.Id;
                item.Click += async (_, _) => await _model.AttachAsync(id);
                _menu.Items.Add(item);
            }
            _menu.Items.Add(new MenuFlyoutSeparator());
        }

        _menu.Items.Add(Item("Add a core…", () => _window.ShowPairWindow()));
        _menu.Items.Add(new MenuFlyoutSeparator());
        _menu.Items.Add(Item("Open the agent's folder", () => Reveal(AppPaths.EngineData)));
        _menu.Items.Add(Item("Open the app's logs", () => Reveal(AppPaths.ShellData)));
        _menu.Items.Add(new MenuFlyoutSeparator());
        _menu.Items.Add(Item("Quit Hi Agent", () => App.Current.Quit()));
    }

    private static MenuFlyoutItem Item(string text, Action action)
    {
        var item = new MenuFlyoutItem { Text = text };
        item.Click += (_, _) => action();
        return item;
    }

    /// <summary>
    /// Show a folder in Explorer. The engine's data directory is the whole
    /// agent, and a person who wants to back it up, copy it to another machine,
    /// or read what it wrote should not have to be told a path.
    /// </summary>
    private static void Reveal(string path)
    {
        try
        {
            Process.Start(new ProcessStartInfo("explorer.exe", $"\"{path}\"") { UseShellExecute = true });
        }
        catch (Exception e)
        {
            Log.Write($"could not open {path}: {e.Message}");
        }
    }

    public void Dispose() => _icon.Dispose();
}

/// <summary>
/// The smallest thing that satisfies <see cref="ICommand"/>. One tray property
/// wants a command and nothing else in this app does, so a binding framework
/// would be scaffolding for a single call site.
/// </summary>
internal static class CommandExtensions
{
    internal static ICommand AsCommand(this Action<object?> action) => new Relay(action);

    private sealed class Relay(Action<object?> action) : ICommand
    {
        public event EventHandler? CanExecuteChanged
        {
            add { }
            remove { }
        }

        public bool CanExecute(object? parameter) => true;

        public void Execute(object? parameter) => action(parameter);
    }
}
