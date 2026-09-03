using System.Text.Json.Serialization;

namespace HiAgent.Windows.Core;

/// <summary>
/// One core this app may attach to: an address, a label, and — in the secure
/// store rather than here — a credential. `docs/arch/topology.md`: "a roster
/// entry is (base URL, credential, label)".
/// </summary>
internal sealed class RosterEntry
{
    /// <summary>Stable local id. Also the key the credential is stored under.</summary>
    public required string Id { get; init; }

    /// <summary>Canonical base URL, as <see cref="CoreClient.NormalizeBaseUrl"/> returned it.</summary>
    public required string BaseUrl { get; set; }

    /// <summary>What the person calls this core. App state; never sent to the core.</summary>
    public required string Label { get; set; }

    /// <summary>
    /// True for the engine this shell starts and supervises. Exactly one entry
    /// may be local: it is this machine, and there is only one of those.
    /// </summary>
    public bool IsLocal { get; init; }

    [JsonIgnore]
    public Uri Uri => new(BaseUrl);
}

/// <summary>What the roster file holds. No secrets — see <see cref="CredentialStore"/>.</summary>
internal sealed class RosterSnapshot
{
    public List<RosterEntry> Entries { get; set; } = [];

    public string? AttachedId { get; set; }
}

/// <summary>The `Set-Cookie` line for the session, kept verbatim.</summary>
internal sealed record SessionCookie(string SetCookieHeader, string Name, string Value);

/// <summary>The body of `POST /api/session`.</summary>
internal sealed record SessionExchange(string Id, string? Credential);

/// <summary>
/// An attached core: where it is, and the session the face will carry.
///
/// The cookie is null for the local engine, and that is not an omission. The
/// core's loopback listener is ungated by construction — `docs/arch/topology.md`
/// § *What is gated* — so exchanging a credential to reach `127.0.0.1` would be
/// the shell authenticating to a door that is open. That reasoning is what
/// deleted `crates/hi-app`; repeating the exchange here would be repeating the
/// mistake in a second language.
/// </summary>
internal sealed record CoreSession(RosterEntry Entry, SessionCookie? Cookie);

internal enum HealthState
{
    /// <summary>`200` — the process answers.</summary>
    Here,

    /// <summary>`503` — reachable, not ready.</summary>
    Asleep,

    /// <summary>Answered, but not in a way `/healthz` is documented to.</summary>
    Unknown,

    /// <summary>Nothing answered.</summary>
    Unreachable,
}

/// <summary>
/// What the window is showing. One enum rather than a handful of booleans,
/// because the states are exclusive and every pair of booleans eventually
/// represents a state that cannot happen.
/// </summary>
internal enum CoreStage
{
    /// <summary>No core in the roster at all — first run, before the engine is up.</summary>
    Empty,

    /// <summary>Starting the local engine, or exchanging a session.</summary>
    Connecting,

    /// <summary>The face is loaded.</summary>
    Ready,

    /// <summary>Reachable but not answering yet.</summary>
    Waiting,

    /// <summary>Something to tell the person, in <see cref="AppModel.StageDetail"/>.</summary>
    Failed,
}

/// <summary>Thrown by <see cref="CoreClient"/>; the message is shown to the person.</summary>
internal class CoreClientException(string message) : Exception(message)
{
    internal sealed class InvalidAddress(string message) : CoreClientException(message);

    internal sealed class MissingSessionCookie()
        : CoreClientException("The core did not return a session cookie.");

    internal sealed class RequestFailed(string detail) : CoreClientException(detail);

    internal sealed class Rejected(int status, string detail) : CoreClientException(
        detail.Length == 0
            ? $"The core rejected the request (HTTP {status})."
            : $"The core rejected the request (HTTP {status}): {detail}")
    {
        internal int Status { get; } = status;
    }
}
