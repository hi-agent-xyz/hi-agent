using HiAgent.Windows.Core;

namespace HiAgent.Windows;

/// <summary>
/// What the shell knows: which cores exist, which one is attached, and what to
/// show while that is being decided.
///
/// No XAML type appears here. The window reads this and renders; this never
/// reaches into the window. That separation is the same one the native surfaces
/// are supposed to have from the engine, one level down.
/// </summary>
internal sealed class AppModel : IDisposable
{
    private const string LocalCoreId = "local";

    private readonly RosterStore _roster = new();
    private readonly LocalCore _local = new();
    private readonly SemaphoreSlim _attaching = new(1, 1);
    private readonly CancellationTokenSource _stopping = new();

    private Task? _health;

    internal CoreStage Stage { get; private set; } = CoreStage.Connecting;

    /// <summary>A sentence for the person when <see cref="Stage"/> alone will not do.</summary>
    internal string? StageDetail { get; private set; }

    internal CoreSession? Session { get; private set; }

    internal IReadOnlyList<RosterEntry> Roster => _roster.Entries;

    internal RosterEntry? Attached => _roster.Attached();

    /// <summary>Raised on the thread that changed something; the window marshals.</summary>
    internal event Action? StateChanged;

    internal async Task StartAsync()
    {
        _roster.Load();
        _local.Changed += OnLocalChanged;

        var baseUrl = await _local.StartAsync().ConfigureAwait(false);
        if (baseUrl is not null)
        {
            // The local entry is written every start rather than once: the port
            // can differ between runs when 12358 was taken, and a roster holding
            // yesterday's port would point the face at nothing.
            _roster.Put(new RosterEntry
            {
                Id = LocalCoreId,
                BaseUrl = baseUrl.ToString(),
                Label = "This computer",
                IsLocal = true,
            });
        }

        var target = _roster.Attached() ?? _roster.Local() ?? _roster.Entries.FirstOrDefault();
        if (target is null)
        {
            Set(CoreStage.Empty, _local.Failure);
            return;
        }

        await AttachAsync(target.Id).ConfigureAwait(false);
        _health = Task.Run(() => PollHealthAsync(_stopping.Token));
    }

    /// <summary>
    /// Make one core the attached one: get a session if it needs one, and tell
    /// the window to load it.
    /// </summary>
    internal async Task AttachAsync(string id)
    {
        await _attaching.WaitAsync().ConfigureAwait(false);
        try
        {
            var entry = _roster.Find(id);
            if (entry is null)
            {
                Set(CoreStage.Failed, "That agent is no longer in the roster.");
                return;
            }

            _roster.Attach(id);
            Session = null;
            Set(CoreStage.Connecting, null);

            if (entry.IsLocal)
            {
                // Wait for the engine to answer before showing the face. A first
                // run provisions its whole runtime before it is useful, which is
                // minutes, so there is no timeout here — the stage says what is
                // happening and the supervisor says if it died.
                await WaitForHealthAsync(entry, _stopping.Token).ConfigureAwait(false);
                Session = new CoreSession(entry, null);
                Set(CoreStage.Connecting, null);
                return;
            }

            var credential = CredentialStore.Load(entry.Id);
            if (credential is null)
            {
                Set(CoreStage.Failed, $"{entry.Label} has no credential on this computer. Add it again.");
                return;
            }

            try
            {
                var (exchange, cookie) = await CoreClient
                    .ExchangeAsync(entry.Uri, credential, DeviceLabel(), _stopping.Token)
                    .ConfigureAwait(false);
                if (exchange.Credential is { } rotated)
                {
                    CredentialStore.Save(entry.Id, rotated);
                }
                Session = new CoreSession(entry, cookie);
                Set(CoreStage.Connecting, null);
            }
            catch (CoreClientException e)
            {
                Set(CoreStage.Failed, e.Message);
            }
        }
        finally
        {
            _attaching.Release();
        }
    }

    /// <summary>
    /// Add an agent the person typed an address and a one-time code for. The core
    /// tells a one-time code from a credential, so this presents whatever it was
    /// given and stores whatever comes back.
    /// </summary>
    /// <summary>
    /// Ask an agent, by name, to let this machine in.
    ///
    /// Nothing is stored by this call: what comes back is a code to show and a
    /// secret to wait on. The roster only grows once <see cref="JoinAsync"/> lands.
    /// </summary>
    internal async Task<JoinInvitation> AskToJoinAsync(string name)
    {
        var baseUrl = CoreClient.AddressForName(name);
        var ticket = await CoreClient
            .AskToJoinAsync(baseUrl, DeviceLabel(), _stopping.Token)
            .ConfigureAwait(false);
        return new JoinInvitation(baseUrl, AgentLabel(name, baseUrl), ticket.Code, ticket.Secret);
    }

    /// <summary>
    /// Wait out one invitation, and take the credential the moment it is approved.
    ///
    /// Polls until answered or cancelled. Past the request's own ten-minute life
    /// the agent answers `expired` and this throws, which is the same clock rather
    /// than a second one kept here.
    /// </summary>
    internal async Task JoinAsync(JoinInvitation invitation, CancellationToken token = default)
    {
        while (true)
        {
            token.ThrowIfCancellationRequested();
            var state = await CoreClient
                .PollJoinAsync(invitation.BaseUrl, invitation.Secret, token)
                .ConfigureAwait(false);
            switch (state)
            {
                case JoinState.Approved approved:
                    // From here it is the ordinary add: the credential goes to the
                    // credential store, the agent joins the roster, and the window
                    // opens it.
                    await AddCoreAsync(
                            invitation.BaseUrl.ToString(),
                            approved.Credential,
                            invitation.Label)
                        .ConfigureAwait(false);
                    return;
                case JoinState.Denied:
                    throw new CoreClientException.RequestFailed("That was turned down.");
                case JoinState.Expired:
                    throw new CoreClientException.RequestFailed(
                        "Nobody answered in time. Ask again.");
                default:
                    await Task.Delay(TimeSpan.FromSeconds(2), token).ConfigureAwait(false);
                    break;
            }
        }
    }

    /// <summary>
    /// What to call an agent added by name. The name itself when that is what was
    /// typed — "iloahz" reads better in the roster than "iloahz.hi-agent.xyz" — and
    /// the host when somebody pasted a whole address.
    /// </summary>
    private static string AgentLabel(string name, Uri baseUrl)
    {
        var bare = name.Trim().ToLowerInvariant().TrimStart('@');
        return bare.Length > 0 && !bare.Contains('.') && !bare.Contains('/')
            ? bare
            : baseUrl.Host;
    }

    /// <summary>
    /// Exchange a one-time code — or a credential a <see cref="JoinAsync"/> just
    /// claimed — and remember the agent.
    /// </summary>
    internal async Task AddCoreAsync(string address, string pairingCode, string label)
    {
        var uri = CoreClient.NormalizeBaseUrl(address);
        var (exchange, cookie) = await CoreClient
            .ExchangeAsync(uri, pairingCode.Trim(), DeviceLabel(), _stopping.Token)
            .ConfigureAwait(false);

        // The core's surface id is the roster key, so re-adding the same core
        // updates one entry instead of growing a second.
        var entry = new RosterEntry
        {
            Id = exchange.Id,
            BaseUrl = uri.ToString(),
            Label = string.IsNullOrWhiteSpace(label) ? uri.Host : label.Trim(),
            IsLocal = false,
        };

        if (exchange.Credential is { } credential)
        {
            CredentialStore.Save(entry.Id, credential);
        }
        else if (CredentialStore.Load(entry.Id) is null)
        {
            // A pairing code always mints a credential; a null here means what
            // was presented already was one, for a core this machine has since
            // forgotten. Nothing to store and nothing that will work later.
            throw new CoreClientException.RequestFailed(
                "That agent returned no credential. Ask it to let this machine in again.");
        }

        _roster.Put(entry);
        _roster.Attach(entry.Id);
        Session = new CoreSession(entry, cookie);
        Set(CoreStage.Connecting, null);
    }

    /// <summary>Forget a core, and fall back to whatever is left.</summary>
    internal async Task ForgetAsync(string id)
    {
        var wasAttached = _roster.AttachedId == id;
        _roster.Remove(id);
        if (!wasAttached)
        {
            StateChanged?.Invoke();
            return;
        }
        var next = _roster.Local() ?? _roster.Entries.FirstOrDefault();
        if (next is null)
        {
            Session = null;
            Set(CoreStage.Empty, null);
            return;
        }
        await AttachAsync(next.Id).ConfigureAwait(false);
    }

    /// <summary>
    /// The face met a 401. Exchange the credential again and hand the window a
    /// new session; the local core has no session to renew, so a 401 there is a
    /// real error rather than an expiry.
    /// </summary>
    internal async Task RenewSessionAsync()
    {
        if (_roster.Attached() is not { IsLocal: false } entry)
        {
            Set(CoreStage.Failed, "The agent refused a request from its own face.");
            return;
        }
        await AttachAsync(entry.Id).ConfigureAwait(false);
    }

    /// <summary>The window says the face painted.</summary>
    internal void ReportReady()
    {
        Set(CoreStage.Ready, null);
    }

    /// <summary>The window says the load failed.</summary>
    internal void ReportFailure(string message)
    {
        Set(CoreStage.Failed, message);
    }

    private async Task WaitForHealthAsync(RosterEntry entry, CancellationToken token)
    {
        while (!token.IsCancellationRequested)
        {
            if (await CoreClient.HealthAsync(entry.Uri, token).ConfigureAwait(false) is HealthState.Here)
            {
                return;
            }
            if (_local.Failure is { } failure)
            {
                Set(CoreStage.Failed, failure);
            }
            else
            {
                Set(CoreStage.Connecting, "Starting the agent…");
            }
            try
            {
                await Task.Delay(TimeSpan.FromSeconds(1), token).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return;
            }
        }
    }

    /// <summary>
    /// Poll the attached core. Not a heartbeat for the core's benefit — it is
    /// how the window knows to stop showing a face that is no longer answering.
    /// </summary>
    private async Task PollHealthAsync(CancellationToken token)
    {
        while (!token.IsCancellationRequested)
        {
            try
            {
                await Task.Delay(TimeSpan.FromSeconds(10), token).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return;
            }

            if (_roster.Attached() is not { } entry)
            {
                continue;
            }
            var health = await CoreClient.HealthAsync(entry.Uri, token).ConfigureAwait(false);
            if (health is HealthState.Here)
            {
                if (Stage is CoreStage.Waiting)
                {
                    Set(CoreStage.Connecting, null);
                }
                continue;
            }
            if (Stage is CoreStage.Ready or CoreStage.Connecting)
            {
                Set(CoreStage.Waiting, entry.IsLocal
                    ? "The agent is not answering."
                    : $"{entry.Label} is not answering.");
            }
        }
    }

    private void OnLocalChanged()
    {
        if (_local.Failure is { } failure && _roster.Attached() is { IsLocal: true })
        {
            Set(CoreStage.Failed, failure);
        }
    }

    /// <summary>
    /// What the core calls this device in its list of authorized surfaces. The
    /// machine name, because that is what a person recognises when they come to
    /// revoke one.
    /// </summary>
    private static string DeviceLabel()
    {
        try
        {
            return Environment.MachineName;
        }
        catch
        {
            return "Windows PC";
        }
    }

    private void Set(CoreStage stage, string? detail)
    {
        Stage = stage;
        StageDetail = detail;
        StateChanged?.Invoke();
    }

    public void Dispose()
    {
        _stopping.Cancel();
        _local.Changed -= OnLocalChanged;
        _local.Dispose();
        _stopping.Dispose();
        _attaching.Dispose();
    }
}
