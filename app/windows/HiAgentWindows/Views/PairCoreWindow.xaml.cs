using HiAgent.Windows.Core;
using Microsoft.UI.Xaml;

namespace HiAgent.Windows.Views;

/// <summary>
/// Adding a core: an address and a pairing code, which is the whole of
/// attachment. There is no QR scanner here — a desktop has a keyboard, and the
/// phones' scanners exist because they do not.
/// </summary>
public sealed partial class PairCoreWindow : Window
{
    private readonly AppModel _model;

    internal PairCoreWindow(AppModel model)
    {
        InitializeComponent();
        _model = model;
        Title = "Add a core";
        AppWindow.Resize(new global::Windows.Graphics.SizeInt32(520, 520));
    }

    private void OnCancel(object sender, RoutedEventArgs e) => Close();

    private async void OnAdd(object sender, RoutedEventArgs e)
    {
        SetBusy(true);
        try
        {
            await _model.AddCoreAsync(AddressBox.Text, CodeBox.Text, LabelBox.Text);
            Close();
        }
        catch (CoreClientException ex)
        {
            Show(ex.Message);
        }
        catch (Exception ex)
        {
            Log.Write($"add core: {ex}");
            Show("That core could not be added.");
        }
        finally
        {
            SetBusy(false);
        }
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
