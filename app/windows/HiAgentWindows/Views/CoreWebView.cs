using HiAgent.Windows.Core;
using Microsoft.UI.Xaml.Controls;
using Microsoft.Web.WebView2.Core;

namespace HiAgent.Windows.Views;

/// <summary>
/// The core's face, in WebView2.
///
/// Everything unusual in here is one of two things: a WebView2 default that is
/// wrong for a full-window app face, or something the other two clients get
/// from their platform and this one has to build. The Kotlin and Swift files of
/// the same name are the reference — where this deviates, it says so.
/// </summary>
internal sealed class CoreWebView
{
    /// <summary>
    /// The Windows spelling of the media-gesture trap that cost both phones a
    /// microphone. Chromium gates `AudioContext` — the graph the mic runs
    /// through, and the graph the agent's voice comes out of — behind a user
    /// gesture, and the face builds that context on load where there is no
    /// gesture. Without this flag the camera works, the mic is silently dead,
    /// and the agent never speaks. See the matching notes in `CoreWebView.swift`
    /// (`mediaTypesRequiringUserActionForPlayback`) and `CoreWebView.kt`
    /// (`mediaPlaybackRequiresUserGesture`).
    /// </summary>
    private const string BrowserArguments = "--autoplay-policy=no-user-gesture-required";

    /// <summary>
    /// The face's own `fetch` can meet a 401 long after the page loaded — a
    /// session that expired while the machine was asleep — and that is invisible
    /// to every navigation event. This is the `WKUserScript` /
    /// `addDocumentStartJavaScript` equivalent.
    ///
    /// `AddScriptToExecuteOnDocumentCreated` is not origin-scoped, so the script
    /// checks the origin itself and the C# side checks it again on arrival. What
    /// it can do is bounded anyway: post one fixed string, which asks the shell
    /// to re-exchange a credential the page has never seen.
    /// </summary>
    private const string SessionObserver = """
        (() => {
          if (window.__hiAgentSessionObserverInstalled) return;
          window.__hiAgentSessionObserverInstalled = true;
          const origin = window.location.origin;
          const originalFetch = window.fetch.bind(window);
          window.fetch = async (...args) => {
            const response = await originalFetch(...args);
            if (response.status === 401 && window.location.origin === origin) {
              window.chrome.webview.postMessage('unauthorized');
            }
            return response;
          };
        })();
        """;

    private readonly WebView2 _view;
    private readonly AppModel _model;

    private CoreSession? _installed;
    private bool _initializing;
    private bool _initialized;
    private bool _renewalRequested;

    internal CoreWebView(WebView2 view, AppModel model)
    {
        _view = view;
        _model = model;
        try
        {
            // Let the window's colour show until the face paints, so opening in
            // dark appearance does not flash a white page.
            _view.DefaultBackgroundColor = Microsoft.UI.Colors.Transparent;
        }
        catch (Exception e)
        {
            Log.Write($"webview background: {e.Message}");
        }
    }

    /// <summary>
    /// Called on every render. Loads the model's session if it is not the one
    /// already loaded, and does nothing at all otherwise — a reload on each
    /// state change would restart the conversation's stream for no reason.
    /// </summary>
    internal void Sync()
    {
        if (_model.Session is not { } session)
        {
            return;
        }
        if (_installed is { } current &&
            current.Entry.BaseUrl == session.Entry.BaseUrl &&
            current.Cookie?.Value == session.Cookie?.Value)
        {
            return;
        }
        _ = InstallAsync(session);
    }

    private async Task InstallAsync(CoreSession session)
    {
        try
        {
            await EnsureInitializedAsync();
        }
        catch (Exception e)
        {
            // The one prerequisite the installer does not carry. Evergreen ships
            // with Windows 11 and arrived on Windows 10 with Edge, so this is
            // rare and worth naming rather than reporting as "could not start".
            Log.Write($"WebView2 could not start: {e}");
            _model.ReportFailure(
                "The WebView2 runtime is missing or would not start. Install the Microsoft Edge WebView2 Runtime and open Hi Agent again.");
            return;
        }

        var core = _view.CoreWebView2;
        _installed = session;
        _renewalRequested = false;

        // Empty the jar before filling it, so it never holds two cores' sessions
        // at once. Relayed cores share one origin — `hi-agent.xyz/ana` and
        // `hi-agent.xyz/bob` are the same site to a cookie store, and `Path=`
        // decides only what is *sent* where, not what is readable. Required by
        // the App section of `docs/arch/topology.md`, which is what lets the
        // session live in the page at all.
        core.CookieManager.DeleteAllCookies();

        if (session.Cookie is { } cookie)
        {
            try
            {
                core.CookieManager.AddOrUpdateCookie(BuildCookie(core, session.Entry.Uri, cookie));
            }
            catch (Exception e)
            {
                Log.Write($"session cookie not installed: {e.Message}");
                _model.ReportFailure("The session could not be handed to the face.");
                return;
            }
        }

        core.Navigate(session.Entry.BaseUrl);
    }

    private async Task EnsureInitializedAsync()
    {
        if (_initialized)
        {
            return;
        }
        if (_initializing)
        {
            // Two renders can race the first install. Waiting is enough: the
            // caller re-checks the session it wanted afterwards.
            while (_initializing)
            {
                await Task.Delay(50);
            }
            return;
        }

        _initializing = true;
        try
        {
            var options = new CoreWebView2EnvironmentOptions(BrowserArguments);
            // An explicit profile directory, under the shell's own state. An
            // unpackaged app otherwise writes its browser profile next to the
            // executable, which is inside the install directory the uninstaller
            // deletes.
            var environment = await CoreWebView2Environment.CreateWithOptionsAsync(
                browserExecutableFolder: string.Empty,
                userDataFolder: AppPaths.WebViewData,
                options: options);
            await _view.EnsureCoreWebView2Async(environment);

            var core = _view.CoreWebView2;
            var settings = core.Settings;
            settings.IsZoomControlEnabled = false;
            settings.AreBrowserAcceleratorKeysEnabled = false;
            settings.IsStatusBarEnabled = false;
            settings.IsPasswordAutosaveEnabled = false;
            settings.IsGeneralAutofillEnabled = false;
#if DEBUG
            settings.AreDevToolsEnabled = true;
            settings.AreDefaultContextMenusEnabled = true;
#else
            settings.AreDevToolsEnabled = false;
            // The face is an app surface, not a page to be inspected, saved or
            // printed by its right-click menu.
            settings.AreDefaultContextMenusEnabled = false;
#endif

            core.PermissionRequested += OnPermissionRequested;
            core.NavigationStarting += OnNavigationStarting;
            core.NavigationCompleted += OnNavigationCompleted;
            core.NewWindowRequested += OnNewWindowRequested;
            core.WebMessageReceived += OnWebMessageReceived;
            core.ProcessFailed += OnProcessFailed;

            await core.AddScriptToExecuteOnDocumentCreatedAsync(SessionObserver);
            _initialized = true;
        }
        finally
        {
            _initializing = false;
        }
    }

    /// <summary>
    /// Rebuild the session cookie from the `Set-Cookie` line the core sent.
    ///
    /// This is the one place the Windows client cannot do what the other two do.
    /// iOS and Android hand the raw header to their cookie store unmodified,
    /// precisely so the core keeps ownership of `Path`, `Max-Age` and
    /// `SameSite`. WebView2's `CookieManager` has no raw-header entry point —
    /// only `CreateCookie(name, value, domain, path)` and properties — so the
    /// header is parsed here and every attribute carried across. Kept verbatim
    /// on <see cref="SessionCookie.SetCookieHeader"/> so what was parsed can be
    /// compared with what arrived.
    ///
    /// Anything the core adds later that is not read here is silently dropped,
    /// which is the risk this comment exists to make visible.
    /// </summary>
    private static CoreWebView2Cookie BuildCookie(CoreWebView2 core, Uri baseUrl, SessionCookie cookie)
    {
        var attributes = cookie.SetCookieHeader.Split(';').Skip(1)
            .Select(part => part.Trim())
            .Where(part => part.Length > 0)
            .ToList();

        string? AttributeValue(string name)
        {
            var prefix = name + "=";
            var found = attributes.FirstOrDefault(a =>
                a.StartsWith(prefix, StringComparison.OrdinalIgnoreCase));
            return found?.Substring(prefix.Length).Trim();
        }

        bool HasFlag(string name) => attributes
            .Any(a => a.Equals(name, StringComparison.OrdinalIgnoreCase));

        var path = AttributeValue("Path") ?? "/";
        var created = core.CookieManager.CreateCookie(cookie.Name, cookie.Value, baseUrl.Host, path);
        created.IsSecure = HasFlag("Secure");
        created.IsHttpOnly = HasFlag("HttpOnly");
        created.SameSite = AttributeValue("SameSite")?.ToLowerInvariant() switch
        {
            "strict" => CoreWebView2CookieSameSiteKind.Strict,
            "none" => CoreWebView2CookieSameSiteKind.None,
            _ => CoreWebView2CookieSameSiteKind.Lax,
        };

        // A cookie with no expiry is a session cookie, and WebView2 spells that
        // by leaving `Expires` alone rather than by a sentinel.
        if (AttributeValue("Max-Age") is { } maxAge && double.TryParse(maxAge, out var seconds))
        {
            created.Expires = DateTimeOffset.UtcNow.AddSeconds(seconds).ToUnixTimeSeconds();
        }
        else if (AttributeValue("Expires") is { } expires &&
                 DateTimeOffset.TryParse(expires, out var when))
        {
            created.Expires = when.ToUnixTimeSeconds();
        }

        return created;
    }

    /// <summary>Exact scheme, host and port — the same rule as the iOS `isTrusted`.</summary>
    private bool IsTrusted(string? uri)
    {
        if (_installed is not { } session || uri is null)
        {
            return false;
        }
        if (!Uri.TryCreate(uri, UriKind.Absolute, out var target))
        {
            return false;
        }
        var expected = session.Entry.Uri;
        return string.Equals(target.Scheme, expected.Scheme, StringComparison.OrdinalIgnoreCase) &&
               string.Equals(target.Host, expected.Host, StringComparison.OrdinalIgnoreCase) &&
               target.Port == expected.Port;
    }

    /// <summary>
    /// Camera and microphone, granted only to the attached core's exact origin.
    ///
    /// Windows has no per-app camera consent to check first the way Android
    /// does — the OS privacy settings are a system-wide switch the person owns,
    /// and a denial there surfaces as a failed capture rather than as something
    /// to pre-empt here. So origin is the whole question at this rung.
    /// </summary>
    private void OnPermissionRequested(CoreWebView2 sender, CoreWebView2PermissionRequestedEventArgs args)
    {
        var kind = args.PermissionKind;
        var wanted = kind is CoreWebView2PermissionKind.Camera
            or CoreWebView2PermissionKind.Microphone
            or CoreWebView2PermissionKind.ClipboardRead;

        args.State = wanted && IsTrusted(args.Uri)
            ? CoreWebView2PermissionState.Allow
            : CoreWebView2PermissionState.Deny;

        // Without this the runtime shows its own prompt on top of the face for
        // a decision that has already been made.
        args.Handled = true;
    }

    /// <summary>
    /// A link out of the core's own origin leaves for the browser.
    ///
    /// The face is the whole window with no address bar, so an off-origin page
    /// would render inside the app's chrome wearing its identity. The session
    /// cookie is host-scoped and does not travel, so this is about what the
    /// person is being shown rather than about what leaks.
    /// </summary>
    private void OnNavigationStarting(CoreWebView2 sender, CoreWebView2NavigationStartingEventArgs args)
    {
        if (IsTrusted(args.Uri))
        {
            return;
        }
        args.Cancel = true;
        OpenExternally(args.Uri);
    }

    private void OnNewWindowRequested(CoreWebView2 sender, CoreWebView2NewWindowRequestedEventArgs args)
    {
        args.Handled = true;
        OpenExternally(args.Uri);
    }

    /// <summary>
    /// Unlike Android, Windows can read a main-frame status: WebView2 reports
    /// it here, so a 401 is met by re-exchanging the credential instead of
    /// showing the person a raw "unauthorized" body.
    /// </summary>
    private void OnNavigationCompleted(CoreWebView2 sender, CoreWebView2NavigationCompletedEventArgs args)
    {
        if (args.IsSuccess && args.HttpStatusCode is >= 200 and < 400)
        {
            _renewalRequested = false;
            _model.ReportReady();
            return;
        }

        if (args.HttpStatusCode == 401)
        {
            RequestRenewal();
            return;
        }

        _model.ReportFailure(args.HttpStatusCode > 0
            ? $"The core answered with HTTP {args.HttpStatusCode}."
            : Describe(args.WebErrorStatus));
    }

    private void OnWebMessageReceived(CoreWebView2 sender, CoreWebView2WebMessageReceivedEventArgs args)
    {
        if (!IsTrusted(args.Source))
        {
            return;
        }
        string message;
        try
        {
            message = args.TryGetWebMessageAsString();
        }
        catch
        {
            return;
        }
        if (message == "unauthorized")
        {
            RequestRenewal();
        }
    }

    private void OnProcessFailed(CoreWebView2 sender, CoreWebView2ProcessFailedEventArgs args)
    {
        Log.Write($"webview process failed: {args.ProcessFailedKind}");
        _model.ReportFailure("The face stopped responding. Try again.");
    }

    private void RequestRenewal()
    {
        if (_renewalRequested)
        {
            return;
        }
        _renewalRequested = true;
        _ = _model.RenewSessionAsync();
    }

    private static void OpenExternally(string uri)
    {
        if (!Uri.TryCreate(uri, UriKind.Absolute, out var target))
        {
            return;
        }
        _ = global::Windows.System.Launcher.LaunchUriAsync(target);
    }

    private static string Describe(CoreWebView2WebErrorStatus status) => status switch
    {
        CoreWebView2WebErrorStatus.HostNameNotResolved => "The core address could not be found.",
        CoreWebView2WebErrorStatus.ConnectionAborted or
            CoreWebView2WebErrorStatus.ConnectionReset or
            CoreWebView2WebErrorStatus.Disconnected => "The connection to the core was lost.",
        CoreWebView2WebErrorStatus.Timeout => "The core took too long to respond.",
        CoreWebView2WebErrorStatus.CannotConnect => "Nothing answered at the core's address.",
        CoreWebView2WebErrorStatus.ServerUnreachable => "The core could not be reached.",
        CoreWebView2WebErrorStatus.CertificateCommonNameIsIncorrect or
            CoreWebView2WebErrorStatus.CertificateExpired or
            CoreWebView2WebErrorStatus.CertificateIsInvalid or
            CoreWebView2WebErrorStatus.ClientCertificateContainsErrors or
            CoreWebView2WebErrorStatus.CertificateRevoked =>
            "The core's secure connection could not be verified.",
        _ => "The core could not be reached.",
    };
}
