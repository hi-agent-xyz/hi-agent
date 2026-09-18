using System.Net.Http.Json;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace HiAgent.Windows.Core;

/// <summary>
/// The engine's config surface, as a client sees it — `docs/core-shell-config-api.md`.
///
/// **This holds no state and reads no secret.** Every value the Settings window
/// shows comes from a call here and goes back the same way; the shell keeps no
/// copy of a setting and never touches `config.db`. The read surface carries
/// `configured: bool` for a key and never the key itself, which is a property of
/// the engine's DTOs rather than a promise this file keeps.
/// `app/apple/macos/HiSettings.swift` is the same client in Swift, and the two
/// are meant to stay recognisably the same file.
///
/// Every route here is **loopback-gated at the engine** (`src/foundation/server/
/// settings.rs`), so this only ever speaks to the core running on this machine.
/// That is also the only core whose credentials, energy and reachability belong
/// to this computer — a remote agent's settings are its own machine's business.
/// </summary>
internal sealed class SettingsClient(Uri baseUrl)
{
    private static readonly JsonSerializerOptions Json = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        PropertyNameCaseInsensitive = true,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };

    internal Uri BaseUrl { get; } = baseUrl;

    /// <summary>Where signing in begins. A browser trip, so it is a URL and not a call.</summary>
    internal Uri SignInUrl => CoreClient.Endpoint(BaseUrl, "account/link/start");

    internal Task<SettingsSnapshot> GetAsync(CancellationToken token = default) =>
        SendAsync<SettingsSnapshot>(HttpMethod.Get, "api/settings", null, token);

    internal Task<AppearanceState> PutAppearanceAsync(AppearancePatch patch, CancellationToken token = default) =>
        SendAsync<AppearanceState>(HttpMethod.Put, "api/settings/appearance", patch, token);

    internal Task<ReachState> PutRelayAsync(bool on, CancellationToken token = default) =>
        SendAsync<ReachState>(HttpMethod.Put, "api/settings/relay", new RelayPatch(on), token);

    internal Task<ModeEcho> PutModeAsync(string mode, CancellationToken token = default) =>
        SendAsync<ModeEcho>(HttpMethod.Put, "api/settings/mode", new ModePatch(mode), token);

    internal Task<FeatureStatus> PutFeatureAsync(string feature, FeaturePatch patch, CancellationToken token = default) =>
        SendAsync<FeatureStatus>(HttpMethod.Put, $"api/settings/credentials/{feature}", patch, token);

    internal Task<EnergyEcho> RefreshEnergyAsync(CancellationToken token = default) =>
        SendAsync<EnergyEcho>(HttpMethod.Post, "api/account/energy/refresh", null, token);

    /// <summary>
    /// A signed-in "manage account" link, falling back to the plain account page.
    /// A person clicking it has already decided to go there, so a broker that is
    /// slow or down is answered with the page they asked for rather than an error
    /// they did not.
    /// </summary>
    internal async Task<Uri> SubscribeUrlAsync(CancellationToken token = default)
    {
        var fallback = new Uri("https://hi-agent.xyz/account");
        try
        {
            var sub = await SendAsync<SubscribeUrl>(HttpMethod.Get, "api/account/subscribe", null, token)
                .ConfigureAwait(false);
            return Uri.TryCreate(sub.Url, UriKind.Absolute, out var url) ? url : fallback;
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            return fallback;
        }
    }

    private async Task<T> SendAsync<T>(HttpMethod method, string path, object? body, CancellationToken token)
    {
        using var request = new HttpRequestMessage(method, CoreClient.Endpoint(BaseUrl, path));
        if (body is not null)
        {
            // By runtime type, not by `object`: the patches are records, and a
            // root serialized as its declared type would put an empty object on
            // the wire.
            request.Content = JsonContent.Create(body, body.GetType(), options: Json);
        }

        HttpResponseMessage response;
        try
        {
            response = await CoreClient.Http.SendAsync(request, token).ConfigureAwait(false);
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            throw new CoreClientException.RequestFailed(
                e.Message.Length == 0 ? "The agent did not answer." : e.Message);
        }

        using (response)
        {
            if (!response.IsSuccessStatusCode)
            {
                var detail = string.Empty;
                try
                {
                    detail = (await response.Content.ReadAsStringAsync(token).ConfigureAwait(false)).Trim();
                }
                catch
                {
                    // A status with no readable body is still a status.
                }
                throw new CoreClientException.Rejected((int)response.StatusCode, detail);
            }

            try
            {
                var value = await response.Content
                    .ReadFromJsonAsync<T>(Json, token)
                    .ConfigureAwait(false);
                return value ?? throw new FormatException("empty body");
            }
            catch (Exception e) when (e is JsonException or FormatException or NotSupportedException)
            {
                throw new CoreClientException.RequestFailed("The agent answered with something unexpected.");
            }
        }
    }
}

// --- the config API's shapes, mirroring `src/foundation/server/settings.rs` ---
//
// Mutable properties with defaults rather than records: these are read straight
// out of JSON and then handed to a window that redraws from them, and a missing
// field in a snapshot should leave a blank row rather than take the window down.

internal sealed class SettingsSnapshot
{
    public AppearanceState Appearance { get; set; } = new();
    public AccountState Account { get; set; } = new();
    public ReachState Reach { get; set; } = new();
    public AboutState About { get; set; } = new();
}

internal sealed class AppearanceState
{
    public ChoiceSetting Theme { get; set; } = new();
    public ChoiceSetting Language { get; set; } = new();
    public FlagSetting Gestures { get; set; } = new();
}

/// <summary>
/// A picker: what it is, what it could be, and whether changing it is felt now
/// or at the next start. `Applies` is the engine's word, not a guess here — it
/// is what lets the window say "takes effect next time" truthfully.
/// </summary>
internal sealed class ChoiceSetting
{
    public string Value { get; set; } = string.Empty;

    public List<Choice> Options { get; set; } = [];

    public string Applies { get; set; } = "live";
}

internal sealed class Choice
{
    public string Value { get; set; } = string.Empty;

    public string Label { get; set; } = string.Empty;
}

internal sealed class FlagSetting
{
    public bool Value { get; set; }

    public string Applies { get; set; } = "live";
}

/// <summary>
/// Who this core is from anywhere else. `Why` carries the registry's own reason
/// there is no name — most often "sign in first" — and is shown as given, because
/// a blank field on the one screen whose job is to say what this agent is called
/// is worse than no screen.
/// </summary>
internal sealed class ReachState
{
    public FlagSetting Relay { get; set; } = new();

    public string? Handle { get; set; }

    public string? Address { get; set; }

    public string? Why { get; set; }
}

internal sealed class AccountState
{
    public string Mode { get; set; } = ModeManaged;

    public IdentityState Identity { get; set; } = new();

    /// <summary>Null in BYOK mode, and before the first broker poll.</summary>
    public EnergySnapshot? Energy { get; set; }

    public List<FeatureStatus> Features { get; set; } = [];

    internal const string ModeManaged = "xiaoyuanzhu";

    internal const string ModeByok = "byok";
}

internal sealed class IdentityState
{
    public bool SignedIn { get; set; }

    public string? Name { get; set; }

    public string? Email { get; set; }
}

internal sealed class EnergySnapshot
{
    public string Tier { get; set; } = string.Empty;

    public long Remaining { get; set; }

    public long Total { get; set; }

    public string ResetsAt { get; set; } = string.Empty;

    public bool OutOfEnergy { get; set; }
}

/// <summary>One capability's own credentials — whether a key is set, never which.</summary>
internal sealed class FeatureStatus
{
    public string Feature { get; set; } = string.Empty;

    public bool Configured { get; set; }

    public string? Wire { get; set; }

    public List<Choice> Wires { get; set; } = [];

    public string? BaseUrl { get; set; }

    public string? Model { get; set; }
}

internal sealed class AboutState
{
    public string Version { get; set; } = string.Empty;

    public string Website { get; set; } = string.Empty;
}

internal sealed record AppearancePatch(string? Theme, string? Language, bool? Gestures);

internal sealed record RelayPatch(bool Relay);

internal sealed record ModePatch(string Mode);

internal sealed record ModeEcho
{
    public string Mode { get; set; } = string.Empty;
}

/// <summary>
/// A capability's write. A blank or omitted `ApiKey` keeps the stored one, so
/// the window never has to ask for a key it is not allowed to read back.
/// </summary>
internal sealed record FeaturePatch(string? Wire, string? ApiKey, string? BaseUrl, string? Model);

internal sealed record EnergyEcho
{
    public EnergySnapshot? Energy { get; set; }
}

internal sealed record SubscribeUrl
{
    public string Url { get; set; } = string.Empty;

    public bool SignedIn { get; set; }
}

/// <summary>
/// What a person calls each capability. The engine names them by the credential
/// field, which is the right name in a config store and the wrong one on screen.
/// </summary>
internal static class FeatureLabels
{
    private static readonly Dictionary<string, string> Labels = new()
    {
        ["llm"] = "Language model",
        ["stt"] = "Speech-to-text",
        ["tts"] = "Text-to-speech",
        ["vision"] = "Vision",
        ["image"] = "Image",
        ["video"] = "Video",
    };

    internal static string For(string feature) =>
        Labels.TryGetValue(feature, out var label) ? label : feature;
}
