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

    internal sealed class InvalidName()
        : CoreClientException("A name is lowercase letters, digits and hyphens.");

    /// <summary>
    /// Nothing answers at that name — the relay has no such handle, or the core
    /// behind it is asleep. One message, because to the person typing they are
    /// the same thing.
    /// </summary>
    internal sealed class NoSuchAgent()
        : CoreClientException("No agent answers to that name yet.");

    internal sealed class TooManyWaiting() : CoreClientException(
        "Too many devices are already waiting there. Try again in a few minutes.");

    internal sealed class MissingSessionCookie()
        : CoreClientException("That agent did not return a session cookie.");

    internal sealed class RequestFailed(string detail) : CoreClientException(detail);

    internal sealed class Rejected(int status, string detail) : CoreClientException(
        detail.Length == 0
            ? $"That agent rejected the request (HTTP {status})."
            : $"That agent rejected the request (HTTP {status}): {detail}")
    {
        internal int Status { get; } = status;
    }
}

/// <summary>
/// What an agent handed back when this machine asked to be let in: a code to show
/// the person, and a secret to poll with. Neither is stored — the secret is spent
/// once for a credential and dropped.
/// </summary>
internal sealed record JoinTicket(string Code, string Secret);

/// <summary>Where one wait got to.</summary>
internal abstract record JoinState
{
    internal sealed record Waiting : JoinState;

    internal sealed record Approved(string Credential) : JoinState;

    internal sealed record Denied : JoinState;

    internal sealed record Expired : JoinState;
}

/// <summary>One outstanding "let me in", held only while the add window is open.</summary>
internal sealed record JoinInvitation(
    Uri BaseUrl,
    string Label,
    /// Shown here and on the agent's Reach view, for the two to be compared. It
    /// authorizes nothing.
    string Code,
    string Secret);
