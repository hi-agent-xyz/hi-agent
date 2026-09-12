using HiAgent.Windows.Core;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace HiAgent.Windows.Views;

/// <summary>
/// Adding a remote agent, name-first.
///
/// **The name leads because it is the only path that suits this machine.** There
/// is no QR scanner here and there never was — a desktop has a keyboard, and the
/// phones' scanners exist because they do not. But a keyboard was never the
/// problem: reading a one-time code off *another* screen means walking to it.
/// Asking needs a name and somebody to say yes.
///
/// The address-and-code form is still here, folded away, for a self-hosted core
/// with no name in the default zone.
/// </summary>
public sealed partial class AddAgentWindow : Window
{
    private readonly AppModel _model;
    private CancellationTokenSource? _waiting;

    internal AddAgentWindow(AppModel model)
    {
        InitializeComponent();
        _model = model;
        Title = "Add a remote hi-agent";
        ZoneText.Text = $".{CoreClient.DefaultZone}";
        AppWindow.Resize(new global::Windows.Graphics.SizeInt32(520, 560));
        // Closing the window is what ends a wait; the request expires on its own
        // at the agent, so there is nothing else to tear down.
        Closed += (_, _) => _waiting?.Cancel();
    }

    /// <summary>
    /// The zone is shown only while what is typed is still a bare label, which is
    /// the only case <see cref="CoreClient.AddressForName"/> appends it in. A field
    /// reading `example.com.hi-agent.xyz` describes a request nobody is about to
    /// make.
    /// </summary>
    private void OnNameChanged(object sender, TextChangedEventArgs e)
    {
        var bare = NameBox.Text.Trim().TrimStart('@');
        ZoneText.Visibility = bare.Contains('.') || bare.Contains('/')
            ? Visibility.Collapsed
            : Visibility.Visible;
    }

    private void OnCancel(object sender, RoutedEventArgs e) => Close();

    /// <summary>
    /// One button for both paths, because only one of them is ever the thing being
    /// filled in: the address and code when that form is open and filled, the name
    /// otherwise.
    /// </summary>
    private async void OnAdd(object sender, RoutedEventArgs e)
    {
        var usingAddress = AddressExpander.IsExpanded && AddressBox.Text.Trim().Length > 0;
        SetBusy(true);
        try
        {
            if (usingAddress)
            {
                await _model.AddCoreAsync(AddressBox.Text, CodeBox.Text, LabelBox.Text);
                Close();
                return;
            }

            var invitation = await _model.AskToJoinAsync(NameBox.Text);

            // Nothing for this person to do now but read the number out, so the
            // form goes away and the number is the window.
            NamingPanel.Visibility = Visibility.Collapsed;
            AddButton.Visibility = Visibility.Collapsed;
            ShownCode.Text = invitation.Code;
            WaitingFor.Text =
                $"Open Reach on {invitation.Label} and let this computer in. " +
                "Check the code matches.";
            WaitingPanel.Visibility = Visibility.Visible;

            _waiting = new CancellationTokenSource();
            await _model.JoinAsync(invitation, _waiting.Token);
            Close();
        }
        catch (OperationCanceledException)
        {
            // The window closed under the wait.
        }
        catch (CoreClientException ex)
        {
            BackToTheForm();
            Show(ex.Message);
        }
        catch (Exception ex)
        {
            Log.Write($"add agent: {ex}");
            BackToTheForm();
            Show("That agent could not be added.");
        }
        finally
        {
            SetBusy(false);
        }
    }

    private void BackToTheForm()
    {
        NamingPanel.Visibility = Visibility.Visible;
        AddButton.Visibility = Visibility.Visible;
        WaitingPanel.Visibility = Visibility.Collapsed;
    }

    private void SetBusy(bool busy)
    {
        Busy.IsActive = busy;
        AddButton.IsEnabled = !busy;
        if (busy)
        {
            ErrorText.Visibility = Visibility.Collapsed;
        }
    }

    private void Show(string message)
    {
        ErrorText.Text = message;
        ErrorText.Visibility = Visibility.Visible;
    }
}
