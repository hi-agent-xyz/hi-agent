using System.Net;
using System.Net.Http.Headers;
using System.Net.Http.Json;
using System.Net.Sockets;
using System.Text.Json;
using System.Text.RegularExpressions;

namespace HiAgent.Windows.Core;

/// <summary>
/// Everything this app says to a core, and the only place an address is parsed.
///
/// The wire is `docs/api/client.md`: `POST /api/session` exchanges a pairing
/// code or a long-lived credential for a short session cookie, and
/// `GET /healthz` says whether the process answers. Nothing here is
/// Windows-specific except which HTTP stack does the sending — the Kotlin and
/// Swift files of the same name do the same things in the same order.
/// </summary>
internal static partial class CoreClient
{
    internal const string SessionCookieName = "hi_surface";

    /// <summary>
    /// No cookie jar. The session belongs to WebView2's store, and a second
    /// copy here would be a second place for it to be stale. iOS makes the same
    /// choice with an ephemeral `URLSession`, Android with `CookieJar.NO_COOKIES`.
    /// </summary>
    private static readonly HttpClient Http = new(new HttpClientHandler
    {
        UseCookies = false,
        AllowAutoRedirect = true,
    })
    {
        Timeout = TimeSpan.FromSeconds(20),
    };

    /// <summary>
    /// Parse and canonicalise an address the person typed, and decide whether we
    /// are allowed to dial it at all.
    ///
    /// Windows imposes no App Transport Security, so unlike iOS and Android
    /// nothing here is forced. The rule is kept anyway, and identically: plain
    /// `http://` reaches a core on this network and never a public host. A
    /// desktop is not the place to be the lax client, and the rule is a client
    /// contract in `docs/api/client.md` terms rather than a platform
    /// accommodation.
    ///
    /// The consequence worth knowing is the same one the phones have: a LAN
    /// address like `http://192.168.1.5:12358` is not a secure context, so the
    /// face gets no microphone and no camera there. `http://127.0.0.1` does.
    ///
    /// Deliberately no DNS: a name is judged by its shape, never by resolving
    /// it. Resolution would block, and a name that resolves inside the LAN today
    /// is not a promise about tomorrow.
    /// </summary>
    internal static Uri NormalizeBaseUrl(string raw)
    {
        var value = raw.Trim();
        if (value.Length == 0 || !Uri.TryCreate(value, UriKind.Absolute, out var url))
        {
            throw new CoreClientException.InvalidAddress(
                "Enter an address beginning with http:// or https://.");
        }

        if (url.Scheme != Uri.UriSchemeHttp && url.Scheme != Uri.UriSchemeHttps)
        {
            throw new CoreClientException.InvalidAddress(
                "Enter an address beginning with http:// or https://.");
        }

        if (!string.IsNullOrEmpty(url.UserInfo))
        {
            throw new CoreClientException.InvalidAddress(
                "An address cannot carry a username or password.");
        }

        if (url.Scheme == Uri.UriSchemeHttp && !IsLocalHost(url.Host))
        {
            throw new CoreClientException.InvalidAddress(
                $"Plain http:// only works for an agent on this network. Use https:// to reach {url.Host}.");
        }

        // Query and fragment are dropped and the path reduced to canonical form,
        // so `https://ana.hi-agent.xyz` and `https://ana.hi-agent.xyz/?x=1` are
        // one roster entry rather than two.
        var builder = new UriBuilder(url)
        {
            Query = string.Empty,
            Fragment = string.Empty,
            Path = NormalizedPath(url.AbsolutePath),
        };
        return builder.Uri;
    }

    /// <summary>Whether `http://` to this host is the local-network case.</summary>
    internal static bool IsLocalHost(string host)
    {
        var bare = host.Trim().Trim('[', ']').ToLowerInvariant();
        if (bare.Length == 0)
        {
            return false;
        }
        if (bare is "localhost")
        {
            return true;
        }
        if (bare.EndsWith(".local", StringComparison.Ordinal) ||
            bare.EndsWith(".localhost", StringComparison.Ordinal))
        {
            return true;
        }

        if (ParseLiteral(bare) is { } literal)
        {
            if (IPAddress.IsLoopback(literal))
            {
                return true;
            }
            if (literal.AddressFamily == AddressFamily.InterNetwork)
            {
                var b = literal.GetAddressBytes();
                return b[0] == 10 ||
                       (b[0] == 172 && b[1] >= 16 && b[1] <= 31) ||
                       (b[0] == 192 && b[1] == 168) ||
                       (b[0] == 169 && b[1] == 254);
            }
            // IPv6: link-local `fe80::/10` and unique-local `fc00::/7`.
            return literal.IsIPv6LinkLocal ||
                   (literal.GetAddressBytes()[0] & 0xFE) == 0xFC;
        }

        // A single-label name — `desktop-7f3`, `hi-core` — is only resolvable on
        // the local network, which is exactly the unqualified-hostname case.
        return !bare.Contains('.');
    }

    /// <summary>
    /// Parse a host as an address literal without ever resolving a name.
    /// The shape is checked first so a hostname never reaches a resolver.
    /// </summary>
    private static IPAddress? ParseLiteral(string host)
    {
        var looksIpv4 = Ipv4Shape().IsMatch(host);
        var looksIpv6 = host.Contains(':');
        if (!looksIpv4 && !looksIpv6)
        {
            return null;
        }
        return IPAddress.TryParse(host, out var address) ? address : null;
    }

    [GeneratedRegex(@"^\d{1,3}(\.\d{1,3}){3}$")]
    private static partial Regex Ipv4Shape();

    private static string NormalizedPath(string path)
    {
        var trimmed = path.Trim('/');
        return trimmed.Length == 0 ? "/" : "/" + trimmed;
    }

    /// <summary>
    /// `POST /api/session`. Presents a pairing code the first time and the
    /// stored credential every time after; the core tells the two apart, not us.
    /// </summary>
    internal static async Task<(SessionExchange Exchange, SessionCookie Cookie)> ExchangeAsync(
        Uri baseUrl,
        string presented,
        string label,
        CancellationToken token = default)
    {
        using var request = new HttpRequestMessage(HttpMethod.Post, Endpoint(baseUrl, "api/session"))
        {
            Content = JsonContent.Create(new { label = label.Trim() }),
        };
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", presented);

        HttpResponseMessage response;
        try
        {
            response = await Http.SendAsync(request, token).ConfigureAwait(false);
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            throw new CoreClientException.RequestFailed(
                e.Message.Length == 0 ? "That agent could not be reached." : e.Message);
        }

        using (response)
        {
            var text = string.Empty;
            try
            {
                text = await response.Content.ReadAsStringAsync(token).ConfigureAwait(false);
            }
            catch
            {
                // An unreadable body is still a status worth reporting.
            }

            if (!response.IsSuccessStatusCode)
            {
                throw new CoreClientException.Rejected((int)response.StatusCode, text.Trim());
            }

            SessionExchange exchange;
            try
            {
                using var json = JsonDocument.Parse(text);
                var root = json.RootElement;
                var id = root.GetProperty("id").GetString()
                         ?? throw new FormatException("no id");
                var credential = root.TryGetProperty("credential", out var c) &&
                                 c.ValueKind == JsonValueKind.String
                    ? c.GetString()
                    : null;
                exchange = new SessionExchange(id, string.IsNullOrEmpty(credential) ? null : credential);
            }
            catch
            {
                throw new CoreClientException.RequestFailed(
                    "That agent returned an unexpected session response.");
            }

            if (!response.Headers.TryGetValues("Set-Cookie", out var lines))
            {
                throw new CoreClientException.MissingSessionCookie();
            }

            var raw = lines.FirstOrDefault(line =>
                line.StartsWith(SessionCookieName + "=", StringComparison.Ordinal));
            if (raw is null)
            {
                throw new CoreClientException.MissingSessionCookie();
            }

            var value = raw[(SessionCookieName.Length + 1)..].Split(';', 2)[0];
            if (value.Length == 0)
            {
                throw new CoreClientException.MissingSessionCookie();
            }

            return (exchange, new SessionCookie(raw, SessionCookieName, value));
        }
    }

    /// <summary>
    /// Where a name without a dot in it lives. An agent's name is a label in this
    /// zone, which is why the add window draws the zone beside the field rather
    /// than asking anybody to type it.
    /// </summary>
    internal const string DefaultZone = "hi-agent.xyz";

    /// <summary>
    /// The address an agent's name resolves to — `iloahz` is
    /// `https://iloahz.hi-agent.xyz`.
    ///
    /// Something with a dot or a scheme in it is taken as a whole address rather
    /// than turned into a nonsense third-level name: a person who types a dot
    /// means an address, and a self-hosted core has one. It then faces the same
    /// cleartext rules anything pasted does.
    /// </summary>
    internal static Uri AddressForName(string raw)
    {
        var name = raw.Trim().ToLowerInvariant().TrimStart('@').Trim('/');
        if (name.Length == 0)
        {
            throw new CoreClientException.InvalidName();
        }
        if (name.Contains("://", StringComparison.Ordinal))
        {
            return NormalizeBaseUrl(name);
        }
        if (name.Contains('.', StringComparison.Ordinal))
        {
            return NormalizeBaseUrl($"https://{name}");
        }
        // The same rule the core states under the name field on its own Reach view.
        foreach (var c in name)
        {
            if (!(char.IsAsciiLetterLower(c) || char.IsAsciiDigit(c) || c == '-'))
            {
                throw new CoreClientException.InvalidName();
            }
        }
        return NormalizeBaseUrl($"https://{name}.{DefaultZone}");
    }

    /// <summary>
    /// `POST /api/access/request` — ask an agent to let this machine in.
    ///
    /// Open at the core, because the caller is by definition a device with no way
    /// in. What comes back is a code to show the person and a secret to poll with —
    /// no access yet, and nothing worth storing.
    /// </summary>
    internal static async Task<JoinTicket> AskToJoinAsync(
        Uri baseUrl,
        string label,
        CancellationToken token = default)
    {
        using var request = new HttpRequestMessage(
            HttpMethod.Post,
            Endpoint(baseUrl, "api/access/request"))
        {
            Content = JsonContent.Create(new { label = label.Trim() }),
        };

        HttpResponseMessage response;
        try
        {
            response = await Http.SendAsync(request, token).ConfigureAwait(false);
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            throw new CoreClientException.RequestFailed(
                e.Message.Length == 0 ? "That agent could not be reached." : e.Message);
        }

        using (response)
        {
            var text = string.Empty;
            try
            {
                text = await response.Content.ReadAsStringAsync(token).ConfigureAwait(false);
            }
            catch
            {
                // An unreadable body is still a status worth reporting.
            }

            // A name nobody has claimed reaches the relay and stops there; an
            // address with no core awake behind it answers 503. To the person
            // typing, both mean the same thing: nothing is there under that name.
            if (response.StatusCode is HttpStatusCode.NotFound or HttpStatusCode.ServiceUnavailable)
            {
                throw new CoreClientException.NoSuchAgent();
            }
            if (response.StatusCode == HttpStatusCode.TooManyRequests)
            {
                throw new CoreClientException.TooManyWaiting();
            }
            if (!response.IsSuccessStatusCode)
            {
                throw new CoreClientException.Rejected((int)response.StatusCode, text.Trim());
            }

            try
            {
                using var json = JsonDocument.Parse(text);
                var root = json.RootElement;
                var code = root.GetProperty("code").GetString()
                           ?? throw new FormatException("no code");
                var secret = root.GetProperty("secret").GetString()
                             ?? throw new FormatException("no secret");
                return new JoinTicket(code, secret);
            }
            catch (Exception e) when (e is JsonException or KeyNotFoundException or FormatException)
            {
                throw new CoreClientException.RequestFailed(
                    "That agent returned an unexpected answer.");
            }
        }
    }

    /// <summary>
    /// `GET /api/access/request` — read where one request got to, presenting the
    /// secret that came back with it.
    /// </summary>
    internal static async Task<JoinState> PollJoinAsync(
        Uri baseUrl,
        string secret,
        CancellationToken token = default)
    {
        using var request = new HttpRequestMessage(
            HttpMethod.Get,
            Endpoint(baseUrl, "api/access/request"));
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", secret);

        HttpResponseMessage response;
        try
        {
            response = await Http.SendAsync(request, token).ConfigureAwait(false);
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            throw new CoreClientException.RequestFailed(
                e.Message.Length == 0 ? "That agent could not be reached." : e.Message);
        }

        using (response)
        {
            if (!response.IsSuccessStatusCode)
            {
                throw new CoreClientException.RequestFailed("That agent stopped answering.");
            }

            var text = await response.Content.ReadAsStringAsync(token).ConfigureAwait(false);
            try
            {
                using var json = JsonDocument.Parse(text);
                var root = json.RootElement;
                var state = root.TryGetProperty("state", out var s) ? s.GetString() : null;
                switch (state)
                {
                    case "approved":
                        var credential = root.TryGetProperty("credential", out var c) &&
                                         c.ValueKind == JsonValueKind.String
                            ? c.GetString()
                            : null;
                        if (string.IsNullOrEmpty(credential))
                        {
                            throw new CoreClientException.RequestFailed(
                                "That agent approved without a credential.");
                        }
                        return new JoinState.Approved(credential);
                    case "denied":
                        return new JoinState.Denied();
                    case "expired":
                        return new JoinState.Expired();
                    default:
                        return new JoinState.Waiting();
                }
            }
            catch (JsonException)
            {
                throw new CoreClientException.RequestFailed(
                    "That agent returned an unexpected answer.");
            }
        }
    }

    /// <summary>`GET /healthz` — open, and the only thing the roster polls.</summary>
    internal static async Task<HealthState> HealthAsync(Uri baseUrl, CancellationToken token = default)
    {
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(token);
        timeout.CancelAfter(TimeSpan.FromSeconds(4));
        try
        {
            using var response = await Http
                .GetAsync(Endpoint(baseUrl, "healthz"), HttpCompletionOption.ResponseHeadersRead, timeout.Token)
                .ConfigureAwait(false);
            return (int)response.StatusCode switch
            {
                200 => HealthState.Here,
                503 => HealthState.Asleep,
                _ => HealthState.Unknown,
            };
        }
        catch
        {
            return HealthState.Unreachable;
        }
    }

    /// <summary>
    /// Append a path to the core's base. A core is at the root of its own origin,
    /// so this is ordinary joining — but a self-hosted one may sit behind
    /// somebody's own reverse proxy at a path, and a base that carries one keeps
    /// it.
    /// </summary>
    internal static Uri Endpoint(Uri baseUrl, string path)
    {
        var basePath = baseUrl.AbsolutePath.Trim('/');
        var joined = string.Join('/', new[] { basePath, path.Trim('/') }.Where(p => p.Length > 0));
        return new UriBuilder(baseUrl)
        {
            Path = "/" + joined,
            Query = string.Empty,
            Fragment = string.Empty,
        }.Uri;
    }
}
