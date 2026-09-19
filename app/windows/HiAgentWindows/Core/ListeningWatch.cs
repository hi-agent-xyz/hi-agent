using System.Text.Json;

namespace HiAgent.Windows.Core;

/// <summary>
/// Holds `GET /api/listening` open and says whether the agent has its ear open.
///
/// True only while somebody is holding the attention key — the right Ctrl on this
/// platform — past the point where the microphone it opened starts being processed.
/// `src/body/gesture.rs` is what decides that; `src/foundation/server/listening.rs`
/// is why it is a fact to read rather than a call pushed at this app.
///
/// **This watches the engine on this machine and no other.** The key is on this
/// keyboard, and the only process that can hear it is the one running here — a remote
/// agent has no way to know it was pressed.
///
/// The tray is the only thing that draws this today, and the tray is the reason it is a
/// subscription rather than a poll: a hold lasts a second or two, so anything on a
/// clock would show it late, briefly, or not at all.
/// </summary>
internal sealed class ListeningWatch : IDisposable
{
    /// <summary>
    /// **Not <see cref="CoreClient.Http"/>.** That client has a 20-second timeout,
    /// which is correct for a request that answers and fatal for one that is supposed
    /// to stay open all day — it would abort the subscription every 20 seconds and the
    /// reconnect loop below would hide it as a flicker. So this keeps its own client
    /// with no timeout at all, and the cancellation token is what ends a read.
    /// </summary>
    private static readonly HttpClient Subscriber = new(new HttpClientHandler { UseCookies = false })
    {
        Timeout = Timeout.InfiniteTimeSpan,
    };

    /// <summary>How long to wait before dialling again after the stream ends.</summary>
    private static readonly TimeSpan Retry = TimeSpan.FromSeconds(3);

    private readonly CancellationTokenSource _stopping = new();
    private Task? _loop;

    /// <summary>Raised on the reading thread with the new value; the window marshals.</summary>
    internal event Action<bool>? Changed;

    /// <summary>The last thing the engine said. False until it says otherwise.</summary>
    internal bool IsListening { get; private set; }

    /// <summary>
    /// Start watching. Safe to call once the local engine has an address; before that
    /// there is nothing to dial. Returns immediately — the loop runs until disposed.
    /// </summary>
    internal void Start(Uri baseUrl)
    {
        _loop ??= Task.Run(() => RunAsync(baseUrl, _stopping.Token));
    }

    private async Task RunAsync(Uri baseUrl, CancellationToken token)
    {
        var endpoint = CoreClient.Endpoint(baseUrl, "api/listening");
        while (!token.IsCancellationRequested)
        {
            try
            {
                await ReadAsync(endpoint, token).ConfigureAwait(false);
            }
            catch (OperationCanceledException) when (token.IsCancellationRequested)
            {
                return;
            }
            catch (Exception e)
            {
                // An engine that is restarting, or not up yet, is the ordinary case
                // here rather than an error — the loop just tries again. Logged at
                // most once per drop, which is what the retry delay bounds it to.
                Log.Write($"listening watch: {e.Message}");
            }

            // The stream ended, so whatever it last said is no longer being kept up to
            // date. An ear reported open and then left that way by a dead socket is the
            // one wrong answer that matters, so close it here rather than on reconnect.
            Set(false);

            try
            {
                await Task.Delay(Retry, token).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return;
            }
        }
    }

    /// <summary>
    /// One connection's worth of frames. Returns when the stream ends; the caller
    /// reconnects.
    ///
    /// Server-sent events are line-oriented: `event:` names the kind, `data:` carries
    /// the payload, and a blank line ends one event. Only `data:` is read — there is
    /// one kind of event on this endpoint, so matching the name as well would be a
    /// second thing to keep in step for no gain. Comment lines (`:`) are the
    /// keep-alive and are skipped by the same rule.
    /// </summary>
    private async Task ReadAsync(Uri endpoint, CancellationToken token)
    {
        using var request = new HttpRequestMessage(HttpMethod.Get, endpoint);
        request.Headers.Accept.ParseAdd("text/event-stream");

        using var response = await Subscriber
            .SendAsync(request, HttpCompletionOption.ResponseHeadersRead, token)
            .ConfigureAwait(false);
        response.EnsureSuccessStatusCode();

        await using var body = await response.Content.ReadAsStreamAsync(token).ConfigureAwait(false);
        using var reader = new StreamReader(body);

        while (!token.IsCancellationRequested)
        {
            var line = await reader.ReadLineAsync(token).ConfigureAwait(false);
            if (line is null)
            {
                return; // the core closed it
            }
            if (!line.StartsWith("data:", StringComparison.Ordinal))
            {
                continue;
            }
            if (Parse(line.AsSpan(5).Trim().ToString()) is { } listening)
            {
                Set(listening);
            }
        }
    }

    /// <summary>
    /// Read one frame's payload. Null for anything unreadable: a shell that threw on a
    /// frame it did not understand would drop the subscription over a field the core
    /// added later.
    /// </summary>
    private static bool? Parse(string payload)
    {
        try
        {
            using var document = JsonDocument.Parse(payload);
            if (!document.RootElement.TryGetProperty("listening", out var value))
            {
                return null;
            }
            return value.ValueKind switch
            {
                JsonValueKind.True => true,
                JsonValueKind.False => false,
                // `GetBoolean` throws on anything else, and a frame that carried a
                // string here would otherwise take the subscription down with it.
                _ => null,
            };
        }
        catch (JsonException)
        {
            return null;
        }
    }

    private void Set(bool listening)
    {
        if (IsListening == listening)
        {
            return;
        }
        IsListening = listening;
        Changed?.Invoke(listening);
    }

    public void Dispose()
    {
        _stopping.Cancel();
        _stopping.Dispose();
    }
}
