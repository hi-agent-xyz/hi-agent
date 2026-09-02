package com.xiaoyuanzhu.hiagent.android.core

import java.net.Inet4Address
import java.net.Inet6Address
import java.net.InetAddress
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject

/**
 * Everything this app says to a core, and the only place an address is parsed.
 *
 * The wire is [docs/api/client.md](../../../../../../../../../../docs/api/client.md):
 * `POST /api/session` exchanges a pairing code or a long-lived credential for a
 * short session cookie, and `GET /healthz` says whether the process answers.
 * Nothing else here is core-specific, and nothing here is Android-specific
 * except which HTTP stack does the sending.
 */
sealed class CoreClientException(message: String) : Exception(message) {
    class InvalidAddress(message: String) : CoreClientException(message)

    object InvalidResponse :
        CoreClientException("The core returned an invalid response.") {
        private fun readResolve(): Any = InvalidResponse
    }

    object MissingSessionCookie :
        CoreClientException("The core did not return a session cookie.") {
        private fun readResolve(): Any = MissingSessionCookie
    }

    class RequestFailed(detail: String) : CoreClientException(detail)

    class Rejected(val status: Int, val detail: String) : CoreClientException(
        if (detail.isEmpty()) {
            "The core rejected the request (HTTP $status)."
        } else {
            "The core rejected the request (HTTP $status): $detail"
        },
    )
}

/** The `Set-Cookie` line for the session, kept verbatim. */
data class SessionCookie(
    /** Exactly what the core sent, handed to `CookieManager` unmodified. */
    val setCookieHeader: String,
    val value: String,
    /** Epoch millis, or null when the core sent no expiry. */
    val expiresAt: Long?,
)

data class SessionExchange(val id: String, val credential: String?)

object CoreClient {
    private const val SESSION_COOKIE_NAME = "hi_surface"
    private val jsonMediaType = "application/json".toMediaType()

    /**
     * Cookies are installed into the WebView's own `CookieManager`, so this
     * client must not keep a jar of its own — otherwise the session lives in two
     * places that can disagree. The iOS client makes the same choice with an
     * ephemeral `URLSession` and `httpShouldSetCookies = false`.
     */
    private val http: OkHttpClient = OkHttpClient.Builder()
        .cookieJar(okhttp3.CookieJar.NO_COOKIES)
        .followRedirects(true)
        .build()

    /**
     * Parse and canonicalise an address the person typed or scanned, and decide
     * whether we are allowed to dial it at all.
     *
     * The cleartext rule is the one piece of policy Android cannot express in
     * `network_security_config.xml` — see the long note in that file. `http://`
     * is accepted only where iOS's `NSAllowsLocalNetworking` would accept it:
     * loopback, a private-range literal, a link-local literal, a single-label
     * hostname, or a `.local` name. A public host over plain HTTP is refused
     * here, which is the only rung that can tell the difference.
     *
     * Deliberately no DNS: a name is judged by its shape, never by resolving it.
     * Resolution would block, and a name that resolves to `10.0.0.4` today is
     * not a promise about tomorrow.
     */
    @Throws(CoreClientException::class)
    fun normalizeBaseUrl(raw: String): HttpUrl {
        val value = raw.trim()
        val url = value.toHttpUrlOrNull()
            ?: throw CoreClientException.InvalidAddress(
                "Enter a core address beginning with http:// or https://.",
            )

        if (url.scheme != "http" && url.scheme != "https") {
            throw CoreClientException.InvalidAddress(
                "Enter a core address beginning with http:// or https://.",
            )
        }
        if (url.username.isNotEmpty() || url.password.isNotEmpty()) {
            throw CoreClientException.InvalidAddress(
                "A core address cannot carry a username or password.",
            )
        }
        if (url.scheme == "http" && !isLocalHost(url.host)) {
            throw CoreClientException.InvalidAddress(
                "Plain http:// only works for a core on this network. " +
                    "Use https:// to reach ${url.host}.",
            )
        }

        // Query and fragment are dropped, and the path is reduced to its
        // canonical form, so `https://hi-agent.xyz/ana` and
        // `https://hi-agent.xyz/ana/?x=1` are one roster entry rather than two.
        return url.newBuilder()
            .query(null)
            .fragment(null)
            .encodedPath(normalizedPath(url.encodedPath))
            .build()
    }

    /** Whether `http://` to this host is the local-network case iOS also allows. */
    fun isLocalHost(host: String): Boolean {
        val bare = host.trim().trim('[', ']').lowercase()
        if (bare.isEmpty()) return false
        if (bare == "localhost") return true
        if (bare.endsWith(".local") || bare.endsWith(".localhost")) return true

        val literal = parseLiteral(bare)
        if (literal != null) {
            return literal.isLoopbackAddress ||
                literal.isLinkLocalAddress ||
                literal.isSiteLocalAddress ||
                // Unique-local IPv6 (`fc00::/7`) is not covered by
                // `isSiteLocalAddress`, which only knows the deprecated
                // `fec0::/10`.
                (literal is Inet6Address && (literal.address[0].toInt() and 0xFE) == 0xFC)
        }

        // A single-label name — `raspberrypi`, `hi-core` — is only resolvable on
        // the local network, which is exactly ATS's unqualified-hostname case.
        return !bare.contains('.')
    }

    /**
     * Parse a host as an address literal without ever resolving a name.
     * `InetAddress.getByName` would do DNS for anything that is not a literal,
     * so the shape is checked first.
     */
    private fun parseLiteral(host: String): InetAddress? {
        val looksIpv4 = host.matches(Regex("""^\d{1,3}(\.\d{1,3}){3}$"""))
        val looksIpv6 = host.contains(':')
        if (!looksIpv4 && !looksIpv6) return null
        return try {
            InetAddress.getByName(host).takeIf { it is Inet4Address || it is Inet6Address }
        } catch (_: Exception) {
            null
        }
    }

    private fun normalizedPath(path: String): String {
        val trimmed = path.trim('/')
        return if (trimmed.isEmpty()) "/" else "/$trimmed"
    }

    /**
     * `POST /api/session`. Presents a pairing code the first time and the stored
     * credential every time after; the core tells the two apart, not us.
     */
    @Throws(CoreClientException::class)
    suspend fun exchange(
        baseUrl: HttpUrl,
        presented: String,
        label: String,
    ): Pair<SessionExchange, SessionCookie> = withContext(Dispatchers.IO) {
        val body = JSONObject().put("label", label.trim()).toString()
        val request = Request.Builder()
            .url(endpoint(baseUrl, "api/session"))
            .post(body.toRequestBody(jsonMediaType))
            .header("Authorization", "Bearer $presented")
            .build()

        val response = try {
            http.newCall(request).execute()
        } catch (e: Exception) {
            throw CoreClientException.RequestFailed(
                e.message ?: "The core could not be reached.",
            )
        }

        response.use {
            val text = try {
                it.body.string()
            } catch (_: Exception) {
                ""
            }
            if (!it.isSuccessful) {
                throw CoreClientException.Rejected(it.code, text.trim())
            }

            val exchange = try {
                val json = JSONObject(text)
                SessionExchange(
                    id = json.getString("id"),
                    credential = if (json.isNull("credential")) {
                        null
                    } else {
                        json.optString("credential").ifEmpty { null }
                    },
                )
            } catch (_: Exception) {
                throw CoreClientException.RequestFailed(
                    "The core returned an unexpected session response.",
                )
            }

            val raw = it.headers.values("Set-Cookie")
                .firstOrNull { line -> line.startsWith("$SESSION_COOKIE_NAME=") }
                ?: throw CoreClientException.MissingSessionCookie
            val parsed = okhttp3.Cookie.parse(baseUrl, raw)
                ?: throw CoreClientException.MissingSessionCookie

            exchange to SessionCookie(
                setCookieHeader = raw,
                value = parsed.value,
                // OkHttp reports a session cookie as `Long.MAX_VALUE`; that is
                // "no expiry", not "expires in 292 million years".
                expiresAt = parsed.expiresAt.takeIf { at -> at != Long.MAX_VALUE },
            )
        }
    }

    /** `GET /healthz` — open, and the only thing the roster polls. */
    suspend fun health(baseUrl: HttpUrl): HealthState = withContext(Dispatchers.IO) {
        val request = Request.Builder().url(endpoint(baseUrl, "healthz")).get().build()
        val client = http.newBuilder()
            .callTimeout(java.time.Duration.ofSeconds(4))
            .build()
        try {
            client.newCall(request).execute().use {
                when (it.code) {
                    200 -> HealthState.HERE
                    503 -> HealthState.ASLEEP
                    else -> HealthState.UNKNOWN
                }
            }
        } catch (_: Exception) {
            HealthState.UNREACHABLE
        }
    }

    /**
     * Append a path to the core's base, keeping any subpath the base carries —
     * a core lives at `https://hi-agent.xyz/ana`, so its session endpoint is
     * `/ana/api/session` and not `/api/session`.
     */
    fun endpoint(baseUrl: HttpUrl, path: String): HttpUrl {
        val base = baseUrl.encodedPath.trim('/')
        val joined = listOf(base, path).filter { it.isNotEmpty() }.joinToString("/")
        return baseUrl.newBuilder()
            .encodedPath("/$joined")
            .query(null)
            .fragment(null)
            .build()
    }
}
