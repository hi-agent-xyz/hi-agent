using System.Diagnostics;
using HiAgent.Windows.Core;
using HiAgent.Windows.Ui;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace HiAgent.Windows.Views;

/// <summary>
/// Settings, as a client of the engine's config API.
///
/// **This window stores nothing and reads no secret.** Every value on it was
/// fetched from `GET /api/settings` and every change is a small PUT; there is no
/// local copy of a setting to disagree with `config.db`, and an API key is
/// something it can write and never read back. That is the same rule
/// `app/apple/macos/HiSettings.swift` follows, and the four panes are that
/// window's four panes — General, Account, Reach, About — deliberately, down to
/// the wording. Nothing here is a Windows invention.
///
/// Until this existed a Windows install had nowhere to change any of it: the
/// engine's config store was reachable only by editing it by hand.
/// </summary>
public sealed partial class SettingsWindow : Window
{
    private readonly SettingsClient _api;
    private readonly IDisposable _theme;
    private readonly CancellationTokenSource _closing = new();

    /// <summary>The engine's last word. Controls are drawn from this, never from what was typed.</summary>
    private SettingsSnapshot? _snapshot;

    /// <summary>
    /// True while controls are being filled in, so setting a value does not read
    /// as a person changing it and write it straight back.
    /// </summary>
    private bool _populating;

    internal SettingsWindow(Uri core)
    {
        InitializeComponent();
        _api = new SettingsClient(core);

        Title = "Settings";
        AppWindow.Resize(new global::Windows.Graphics.SizeInt32(820, 620));
        _theme = Theme.Bind(Root);

        Closed += (_, _) =>
        {
            _closing.Cancel();
            _theme.Dispose();
        };

        _ = LoadAsync();
    }

    private async Task LoadAsync()
    {
        try
        {
            var snapshot = await _api.GetAsync(_closing.Token);
            _snapshot = snapshot;
            Loading.IsActive = false;
            Loading.Visibility = Visibility.Collapsed;
            Nav.Visibility = Visibility.Visible;
            Populate(snapshot);
        }
        catch (OperationCanceledException)
        {
            // The window closed under the read.
        }
        catch (Exception e)
        {
            Log.Write($"settings load: {e}");
            Loading.IsActive = false;
            Loading.Visibility = Visibility.Collapsed;
            LoadError.Text = e is CoreClientException
                ? $"Settings could not be opened: {e.Message}"
                : "Settings could not be opened. The agent on this computer is not answering.";
            LoadError.Visibility = Visibility.Visible;
        }
    }

    // --- drawing ------------------------------------------------------------

    private void Populate(SettingsSnapshot snapshot)
    {
        _populating = true;
        try
        {
            Fill(ThemeBox, snapshot.Appearance.Theme);
            Fill(LanguageBox, snapshot.Appearance.Language);
            GesturesSwitch.IsOn = snapshot.Appearance.Gestures.Value;

            RenderAccount();
            RenderReach();

            VersionText.Text = $"Version {snapshot.About.Version}";
            WebsiteLink.Content = snapshot.About.Website.Replace("https://", string.Empty);
            WebsiteLink.NavigateUri = Uri.TryCreate(snapshot.About.Website, UriKind.Absolute, out var site)
                ? site
                : null;
        }
        finally
        {
            _populating = false;
        }
    }

    private void RenderAccount()
    {
        if (_snapshot is not { } snapshot)
        {
            return;
        }

        var wasPopulating = _populating;
        _populating = true;
        try
        {
            var account = snapshot.Account;
            var managed = account.Mode != AccountState.ModeByok;
            ModeChoice.SelectedIndex = managed ? 0 : 1;
            ManagedPane.Visibility = managed ? Visibility.Visible : Visibility.Collapsed;
            ByokPane.Visibility = managed ? Visibility.Collapsed : Visibility.Visible;

            SignedInText.Text = account.Identity.SignedIn
                ? $"as {account.Identity.Name ?? account.Identity.Email ?? "your account"}"
                : "Not signed in";
            SignInButton.Visibility = account.Identity.SignedIn ? Visibility.Collapsed : Visibility.Visible;

            PlanText.Text = account.Energy is { } plan
                ? (plan.Tier == "sub" ? "Subscribed" : "Free")
                : "—";
            EnergyText.Text = account.Energy is { } energy
                ? $"{energy.Remaining} / {energy.Total}"
                : "—";

            RenderFeatures();
        }
        finally
        {
            _populating = wasPopulating;
        }
    }

    private void RenderFeatures()
    {
        FeatureList.Children.Clear();
        if (_snapshot is not { } snapshot)
        {
            return;
        }

        foreach (var status in snapshot.Account.Features)
        {
            var row = new StackPanel
            {
                Orientation = Orientation.Horizontal,
                Spacing = 12,
                Padding = new Thickness(0, 6, 0, 6),
            };
            row.Children.Add(new TextBlock
            {
                Text = FeatureLabels.For(status.Feature),
                Width = 150,
                VerticalAlignment = VerticalAlignment.Center,
            });
            row.Children.Add(new TextBlock
            {
                // Whether a key is set is the whole of what can be shown: the
                // engine will not hand one back, by design.
                Text = status.Configured ? "configured" : "not set",
                Opacity = 0.6,
                VerticalAlignment = VerticalAlignment.Center,
            });

            var edit = new HyperlinkButton { Content = "Edit…" };
            var feature = status;
            edit.Click += async (_, _) => await EditFeatureAsync(feature);
            row.Children.Add(edit);

            FeatureList.Children.Add(row);
        }
    }

    private void RenderReach()
    {
        if (_snapshot is not { } snapshot)
        {
            return;
        }

        var wasPopulating = _populating;
        _populating = true;
        try
        {
            var reach = snapshot.Reach;
            RelaySwitch.IsOn = reach.Relay.Value;
            RelayNote.Text = reach.Relay.Value
                ? "This machine holds a connection to the community, so the address below reaches it from anywhere."
                : "Off. The name stays yours — it simply answers “asleep” until this is back on.";

            var named = !string.IsNullOrWhiteSpace(reach.Handle);
            NamedPane.Visibility = named ? Visibility.Visible : Visibility.Collapsed;
            UnnamedPane.Visibility = named ? Visibility.Collapsed : Visibility.Visible;

            if (named)
            {
                HandleText.Text = reach.Handle ?? string.Empty;
                var hasAddress = !string.IsNullOrWhiteSpace(reach.Address);
                AddressRow.Visibility = hasAddress ? Visibility.Visible : Visibility.Collapsed;
                AddressText.Text = reach.Address ?? string.Empty;
                CopyAddressButton.Content = "Copy";
            }
            else
            {
                // The registry's own words, when it has any. Claiming a name is
                // the agent's own Reach view, which can also list and revoke
                // devices; this pane exists so nobody has to ask the agent what
                // it is called.
                ReachWhy.Text = string.IsNullOrWhiteSpace(reach.Why)
                    ? "Ask the agent to show “reach” to claim one."
                    : reach.Why!;
            }
        }
        finally
        {
            _populating = wasPopulating;
        }
    }

    /// <summary>
    /// Fill a picker from the engine's options. A stored value the engine no
    /// longer offers is added rather than silently replaced — a picker that
    /// quietly reads as something else than what is in force is worse than one
    /// showing an unfamiliar word.
    /// </summary>
    private static void Fill(ComboBox box, ChoiceSetting setting)
    {
        box.Items.Clear();
        var selected = -1;
        foreach (var choice in setting.Options)
        {
            box.Items.Add(new ComboBoxItem { Content = choice.Label, Tag = choice.Value });
            if (choice.Value == setting.Value)
            {
                selected = box.Items.Count - 1;
            }
        }
        if (selected < 0 && setting.Value.Length > 0)
        {
            box.Items.Add(new ComboBoxItem { Content = setting.Value, Tag = setting.Value });
            selected = box.Items.Count - 1;
        }
        box.SelectedIndex = selected;
    }

    private static string? SelectedValue(ComboBox box) =>
        (box.SelectedItem as ComboBoxItem)?.Tag as string;

    // --- panes --------------------------------------------------------------

    private void OnPaneChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        if (args.SelectedItem is not NavigationViewItem item)
        {
            return;
        }

        var pane = item.Tag as string ?? "general";
        PaneTitle.Text = item.Content as string ?? "Settings";
        GeneralPane.Visibility = pane == "general" ? Visibility.Visible : Visibility.Collapsed;
        AccountPane.Visibility = pane == "account" ? Visibility.Visible : Visibility.Collapsed;
        ReachPane.Visibility = pane == "reach" ? Visibility.Visible : Visibility.Collapsed;
        AboutPane.Visibility = pane == "about" ? Visibility.Visible : Visibility.Collapsed;
        ErrorText.Visibility = Visibility.Collapsed;
    }

    // --- writes -------------------------------------------------------------

    private async void OnThemeChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_populating || SelectedValue(ThemeBox) is not { } value)
        {
            return;
        }
        // The core persists, the shell applies — so this happens here and not in
        // the engine. Only after the write lands: a change the engine refused
        // must not leave the app painted as something it is not.
        if (await WriteAppearanceAsync(new AppearancePatch(value, null, null)))
        {
            Theme.Apply(value);
        }
    }

    private async void OnLanguageChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_populating || SelectedValue(LanguageBox) is not { } value)
        {
            return;
        }
        await WriteAppearanceAsync(new AppearancePatch(null, value, null));
    }

    private async void OnGesturesToggled(object sender, RoutedEventArgs e)
    {
        if (_populating)
        {
            return;
        }
        await WriteAppearanceAsync(new AppearancePatch(null, null, GesturesSwitch.IsOn));
    }

    private async Task<bool> WriteAppearanceAsync(AppearancePatch patch)
    {
        return await WriteAsync(async token =>
        {
            var next = await _api.PutAppearanceAsync(patch, token);
            _snapshot!.Appearance = next;

            // Whether a change is felt now or at the next start is the engine's
            // answer, carried on every setting it returns. Saying so is the
            // whole reason that field is on the wire.
            var applies =
                patch.Theme is not null ? next.Theme.Applies
                : patch.Language is not null ? next.Language.Applies
                : next.Gestures.Applies;
            if (applies == "restart")
            {
                RestartHint.Visibility = Visibility.Visible;
            }
        });
    }

    private async void OnRelayToggled(object sender, RoutedEventArgs e)
    {
        if (_populating)
        {
            return;
        }
        await WriteAsync(async token =>
        {
            // Reachability applies live, so the answer carries the state back and
            // the pane redraws from it rather than from what was asked for.
            _snapshot!.Reach = await _api.PutRelayAsync(RelaySwitch.IsOn, token);
            RenderReach();
        });
    }

    private async void OnModeChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_populating || (ModeChoice.SelectedItem as FrameworkElement)?.Tag is not string mode)
        {
            return;
        }
        await WriteAsync(async token =>
        {
            var echo = await _api.PutModeAsync(mode, token);
            _snapshot!.Account.Mode = echo.Mode.Length > 0 ? echo.Mode : mode;
            RenderAccount();
        });
    }

    private async void OnRefreshEnergy(object sender, RoutedEventArgs e)
    {
        RefreshEnergyButton.IsEnabled = false;
        try
        {
            await WriteAsync(async token =>
            {
                _snapshot!.Account.Energy = (await _api.RefreshEnergyAsync(token)).Energy;
                RenderAccount();
            });
        }
        finally
        {
            RefreshEnergyButton.IsEnabled = true;
        }
    }

    private async Task EditFeatureAsync(FeatureStatus status)
    {
        var fields = new StackPanel { Spacing = 12, Width = 360 };

        ComboBox? wires = null;
        if (status.Feature == "llm" && status.Wires.Count > 0)
        {
            wires = new ComboBox { Header = "Agent", MinWidth = 200 };
            Fill(wires, new ChoiceSetting
            {
                Value = status.Wire ?? string.Empty,
                Options = status.Wires,
            });
            fields.Children.Add(wires);
        }

        var key = new PasswordBox
        {
            Header = "API key",
            // Never the key itself — the engine does not return one. A blank
            // field keeps whatever is stored, which is what makes editing a base
            // URL possible without re-typing a secret.
            PlaceholderText = status.Configured ? "•••••• (leave blank to keep)" : string.Empty,
        };
        var baseUrl = new TextBox { Header = "Base URL (optional)", Text = status.BaseUrl ?? string.Empty };
        var model = new TextBox { Header = "Model (optional)", Text = status.Model ?? string.Empty };
        fields.Children.Add(key);
        fields.Children.Add(baseUrl);
        fields.Children.Add(model);

        var dialog = new ContentDialog
        {
            // A dialog in WinUI belongs to a XAML tree, not to a window.
            XamlRoot = Root.XamlRoot,
            Title = FeatureLabels.For(status.Feature),
            Content = fields,
            PrimaryButtonText = "Save",
            CloseButtonText = "Cancel",
            DefaultButton = ContentDialogButton.Primary,
        };

        if (await dialog.ShowAsync() != ContentDialogResult.Primary)
        {
            return;
        }

        var typed = key.Password.Trim();
        await WriteAsync(async token =>
        {
            var next = await _api.PutFeatureAsync(status.Feature, new FeaturePatch(
                Wire: wires is null ? null : SelectedValue(wires),
                ApiKey: typed.Length == 0 ? null : typed,
                BaseUrl: baseUrl.Text.Trim(),
                Model: model.Text.Trim()), token);

            var features = _snapshot!.Account.Features;
            var at = features.FindIndex(f => f.Feature == next.Feature);
            if (at >= 0)
            {
                features[at] = next;
            }
            RenderFeatures();
        });
    }

    private void OnSignIn(object sender, RoutedEventArgs e) => Open(_api.SignInUrl);

    private async void OnManageAccount(object sender, RoutedEventArgs e)
    {
        ManageAccountButton.IsEnabled = false;
        try
        {
            Open(await _api.SubscribeUrlAsync(_closing.Token));
        }
        catch (OperationCanceledException)
        {
            // The window closed under the call.
        }
        finally
        {
            ManageAccountButton.IsEnabled = true;
        }
    }

    private void OnCopyAddress(object sender, RoutedEventArgs e)
    {
        var address = AddressText.Text;
        if (address.Length == 0)
        {
            return;
        }
        try
        {
            var package = new global::Windows.ApplicationModel.DataTransfer.DataPackage();
            package.SetText(address);
            global::Windows.ApplicationModel.DataTransfer.Clipboard.SetContent(package);
            CopyAddressButton.Content = "Copied";
        }
        catch (Exception ex)
        {
            Log.Write($"copy address: {ex.Message}");
        }
    }

    /// <summary>
    /// One place a write is made, so one place says what happened when it fails —
    /// and puts the controls back to what the engine last said, rather than
    /// leaving a switch showing a state nothing is in. False when the change did
    /// not land, for the one caller that has something else to undo.
    /// </summary>
    private async Task<bool> WriteAsync(Func<CancellationToken, Task> write)
    {
        ErrorText.Visibility = Visibility.Collapsed;
        try
        {
            await write(_closing.Token);
            return true;
        }
        catch (OperationCanceledException)
        {
            // The window closed under the write.
            return false;
        }
        catch (CoreClientException e)
        {
            Show(e.Message);
            return false;
        }
        catch (Exception e)
        {
            Log.Write($"settings write: {e}");
            Show("That change could not be saved.");
            return false;
        }
    }

    private void Show(string message)
    {
        ErrorText.Text = message;
        ErrorText.Visibility = Visibility.Visible;
        if (_snapshot is { } snapshot)
        {
            Populate(snapshot);
        }
    }

    /// <summary>
    /// Hand a URL to the browser. Signing in and managing an account are the
    /// person's business with the broker, and a webview inside the shell would
    /// be a worse place to do it than the browser they are already signed in to.
    /// </summary>
    private static void Open(Uri url)
    {
        try
        {
            Process.Start(new ProcessStartInfo(url.ToString()) { UseShellExecute = true });
        }
        catch (Exception e)
        {
            Log.Write($"could not open {url}: {e.Message}");
        }
    }
}
