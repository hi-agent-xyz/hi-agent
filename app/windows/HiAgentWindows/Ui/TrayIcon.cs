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
/// **It says what state the agent is in**, because the window that also says so
/// can be closed and this cannot. macOS does it by putting text beside the
/// menu-bar icon when the engine will not start (`⚠ needs setup`,
/// `⚠ startup failed` in `lib.rs`); a notification-area icon has no text beside
/// it, so the same fact arrives here as the tooltip, the first line of the menu,
/// and the icon itself. Hovering is a choice, so the colour is the part that
/// does not need one.
///
/// Three states, matching the menu bar's: the mark in colour while the agent is
/// answering, drained of colour while it is not, and knocked out of coral while
/// it has its ear open — somebody is holding the attention key
/// (<see cref="AppModel.IsListening"/>). The third is the transient one and it
/// wins over the other two, because an agent that is hearing you is plainly
/// there and the question you have while holding a key is only ever whether it
/// is listening.
///
/// It also ends the moment the key comes up, which is *sooner* than the macOS
/// menu bar settles: there the icon holds its colour while the reply is still
/// being read in the text beside it, and there is no text beside this one to
/// wait for. Both are reading the same fact — see
/// `src/foundation/server/listening.rs`.
///
/// Built in code rather than declared in XAML because the window it would be
/// declared in is not shown at launch, and an icon in an unshown window's tree
/// is never created. <c>ForceCreate</c> is the supported way to say that.
/// </summary>
internal sealed class TrayIcon : IDisposable
{
    /// <summary>
    /// The mark in colour, and the same mark drained of it. All three states are
    /// the brand icon at 16/32/192 — `scripts/make-tray-icos.py` derives the
    /// other two from this one, so a change to the mark cannot leave them
    /// showing different logos.
    /// </summary>
    private const string LiveIcon = "ms-appx:///Assets/HiAgent.ico";

    private const string QuietIcon = "ms-appx:///Assets/HiAgentGrey.ico";

    /// <summary>
    /// The same mark again, white on a filled coral tile — the one difference
    /// that still reads at 16px, where a badge or an outline does not.
    /// `scripts/make-tray-icos.py` derives this and the grey one together.
    /// </summary>
    private const string ListeningIcon = "ms-appx:///Assets/HiAgentListening.ico";

    /// <summary>
    /// What `NOTIFYICONDATA.szTip` holds (Vista and later). Longer than this is
    /// the platform's truncation rather than ours, and an ellipsis reads better
    /// than a sentence cut mid-word.
    /// </summary>
    private const int TipLimit = 127;

    private readonly AppModel _model;
    private readonly MainWindow _window;
    private readonly TaskbarIcon _icon;
    private readonly MenuFlyout _menu = new();

    /// <summary>Which of the two icons is up, so an unchanged state is not re-set every poll.</summary>
    private string? _showing;

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

        Sync();
        _icon.ForceCreate();
    }

    /// <summary>Re-read the model. Cheap, and the roster is a handful of entries.</summary>
    internal void Sync()
    {
        // Whether the agent is there, not whether its face has painted — the
        // window can go a whole session without being opened, and the tray still
        // has to be right. <see cref="AppModel.AgentIsHere"/> is where that
        // difference is decided.
        var here = _model.AgentIsHere;
        var listening = _model.IsListening;
        var stage = _model.Stage;

        ShowIcon(listening ? ListeningIcon : here ? LiveIcon : QuietIcon);

        // Always "Hi Agent — what is going on", so the tooltip is worth reading
        // rather than a label that repeats the icon. When all is well, what is
        // going on is which agent this is attached to.
        //
        // Listening is not in the menu, only here and in the icon: a hold lasts a
        // second or two and cannot be held while opening a menu, so a header for
        // it would be a line nobody can ever see.
        var sentence = listening ? "Listening" : here ? null : Sentence(stage);
        _icon.ToolTipText = Clamp(
            $"Hi Agent — {sentence ?? _model.Attached?.Label ?? "running"}");

        Rebuild(listening ? null : sentence);
    }

    /// <summary>
    /// What is going on, in one line. The detail when the model has one — it is
    /// already a sentence written for a person, and wording it twice is how the
    /// two come to disagree — and the stage's own words when it does not.
    /// </summary>
    private string Sentence(CoreStage stage) => _model.StageDetail ?? stage switch
    {
        CoreStage.Empty => "No agent yet",
        CoreStage.Connecting => "Starting the agent…",
        CoreStage.Waiting => "Waiting for the agent",
        CoreStage.Failed => "The agent could not be reached",
        _ => "Running",
    };

    private static string Clamp(string tip) =>
        tip.Length <= TipLimit ? tip : tip[..(TipLimit - 1)] + "…";

    private void ShowIcon(string source)
    {
        if (_showing == source)
        {
            return;
        }
        try
        {
            _icon.IconSource = new BitmapImage(new Uri(source));
            _showing = source;
        }
        catch (Exception e)
        {
            // No icon is survivable — an unnamed blank in the tray still opens
            // the menu. Failing to start over it would not be.
            Log.Write($"tray icon image: {e.Message}");
        }
    }

    private void Rebuild(string? state)
    {
        _menu.Items.Clear();

        if (state is not null)
        {
            // A header, not a command: the menu is where a person looks after the
            // icon has told them something is off, and it should say what without
            // making them open the window to find out.
            _menu.Items.Add(new MenuFlyoutItem { Text = state, IsEnabled = false });
            _menu.Items.Add(new MenuFlyoutSeparator());
        }

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

        // Settings are the local engine's, and there is nothing to open when this
        // install hosts none — a person attached to somebody else's agent changes
        // its settings on the machine running it.
        var settings = Item("Settings…", () => _window.ShowSettingsWindow());
        settings.IsEnabled = _model.LocalCoreUrl is not null;
        _menu.Items.Add(settings);
        _menu.Items.Add(Item("Add an agent…", () => _window.ShowPairWindow()));
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
