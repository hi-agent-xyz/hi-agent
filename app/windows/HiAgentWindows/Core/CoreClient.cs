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
                "Enter a core address beginning with http:// or https://.");
        }

        if (url.Scheme != Uri.UriSchemeHttp && url.Scheme != Uri.UriSchemeHttps)
        {
            throw new CoreClientException.InvalidAddress(
                "Enter a core address beginning with http:// or https://.");
        }

        if (!string.IsNullOrEmpty(url.UserInfo))
        {
            throw new CoreClientException.InvalidAddress(
                "A core address cannot carry a username or password.");
        }

        if (url.Scheme == Uri.UriSchemeHttp && !IsLocalHost(url.Host))
        {
            throw new CoreClientException.InvalidAddress(
                $"Plain http:// only works for a core on this network. Use https:// to reach {url.Host}.");
        }

        // Query and fragment are dropped and the path reduced to canonical form,
        // so `https://hi-agent.xyz/ana` and `https://hi-agent.xyz/ana/?x=1` are
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
                e.Message.Length == 0 ? "The core could not be reached." : e.Message);
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
                    "The core returned an unexpected session response.");
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
    /// Append a path to the core's base, keeping any subpath the base carries —
    /// a core lives at `https://hi-agent.xyz/ana`, so its session endpoint is
    /// `/ana/api/session` and not `/api/session`.
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
