package com.xiaoyuanzhu.hiagent.android.core

import java.net.Inet4Address
import java.net.Inet6Address
import java.net.InetAddress
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MultipartBody
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody
import okhttp3.RequestBody.Companion.toRequestBody
import okio.BufferedSink
import okio.source
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

    object InvalidName :
        CoreClientException("A name is lowercase letters, digits and hyphens.") {
        private fun readResolve(): Any = InvalidName
    }

    object NoSuchAgent :
        CoreClientException("No agent answers to that name yet.") {
        private fun readResolve(): Any = NoSuchAgent
    }

    object TooManyWaiting : CoreClientException(
        "Too many devices are already waiting there. Try again in a few minutes.",
    ) {
        private fun readResolve(): Any = TooManyWaiting
    }

    object InvalidResponse :
        CoreClientException("The agent returned an invalid response.") {
        private fun readResolve(): Any = InvalidResponse
    }

    object MissingSessionCookie :
        CoreClientException("The agent did not return a session cookie.") {
        private fun readResolve(): Any = MissingSessionCookie
    }

    class RequestFailed(detail: String) : CoreClientException(detail)

    class Rejected(val status: Int, val detail: String) : CoreClientException(
        if (detail.isEmpty()) {
            "The agent rejected the request (HTTP $status)."
        } else {
            "The agent rejected the request (HTTP $status): $detail"
        },
    )
}

/**
 * What the agent handed back when this device asked to be let in: a code to show
 * the person, and a secret to poll with.
 *
 * Neither is stored. The secret is spent once for a credential and dropped; if the
 * app closes mid-wait the request expires on its own at the agent.
 */
data class JoinTicket(val code: String, val secret: String)

/** Where one wait got to. */
sealed interface JoinState {
    object Waiting : JoinState

    data class Approved(val credential: String) : JoinState

    object Denied : JoinState

    object Expired : JoinState
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
     * Where a name without a dot in it lives. An agent's name is a label in this
     * zone, which is why the add screen draws the zone beside the field instead of
     * asking anybody to type it — on a television that difference is the difference
     * between six key presses on a remote and forty-three.
     */
    const val DEFAULT_ZONE = "hi-agent.xyz"

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
                "Enter an address beginning with http:// or https://.",
            )

        if (url.scheme != "http" && url.scheme != "https") {
            throw CoreClientException.InvalidAddress(
                "Enter an address beginning with http:// or https://.",
            )
        }
        if (url.username.isNotEmpty() || url.password.isNotEmpty()) {
            throw CoreClientException.InvalidAddress(
                "An address cannot carry a username or password.",
            )
        }
        if (url.scheme == "http" && !isLocalHost(url.host)) {
            throw CoreClientException.InvalidAddress(
                "Plain http:// only works for an agent on this network. " +
                    "Use https:// to reach ${url.host}.",
            )
        }

        // Query and fragment are dropped, and the path is reduced to its
        // canonical form, so `https://ana.hi-agent.xyz` and
        // `https://ana.hi-agent.xyz/?x=1` are one roster entry rather than two.
        return url.newBuilder()
            .query(null)
            .fragment(null)
            .encodedPath(normalizedPath(url.encodedPath))
            .build()
    }

    /**
     * The address an agent's name resolves to — `iloahz` is
     * `https://iloahz.hi-agent.xyz`.
     *
     * Something with a dot or a scheme in it is taken as a whole address rather
     * than turned into a nonsense third-level name: a person who types a dot means
     * an address, and a self-hosted agent has one. That also means the local-network
     * rules in [normalizeBaseUrl] still get their say — typing `hi-core` here
     * reaches the box on this network over plain http, exactly as pasting it would.
     */
    @Throws(CoreClientException::class)
    fun addressForName(raw: String): HttpUrl {
        var name = raw.trim().lowercase().removePrefix("@").trim('/')
        if (name.isEmpty()) throw CoreClientException.InvalidName
        if (name.contains("://")) return normalizeBaseUrl(name)
        // A single label is ambiguous: `hi-core` is a machine on this network and
        // `iloahz` is a name in the zone. The zone wins — a self-hosted core is
        // reached by pasting its address, which is the branch above.
        if (name.contains('.')) return normalizeBaseUrl("https://$name")
        // The same rule the core states under the name field on its own Reach view.
        if (!name.all { it.isDigit() || (it in 'a'..'z') || it == '-' }) {
            throw CoreClientException.InvalidName
        }
        return normalizeBaseUrl("https://$name.$DEFAULT_ZONE")
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
                e.message ?: "The agent could not be reached.",
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
                    "The agent returned an unexpected session response.",
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

    /**
     * Hand files to a core through its `file` channel — `POST /api/in/file`, the
     * same door a drag-drop onto the face goes through.
     *
     * **The bytes are streamed out of the `ContentResolver` and never held.** What
     * arrives from a share is a `content://` URI owned by another app; opening it and
     * copying it into a `ByteArray` first would put a phone video in this process's
     * heap, which is a few hundred megabytes short of working. OkHttp's
     * [RequestBody.writeTo] is given the stream instead, so the upload is a pipe from
     * the other app's file to the socket. The core's half of the arrangement is that
     * `/api/in/file` writes each part through to a blob as it arrives and declares no
     * size limit at all.
     *
     * No `note` part: what was shared is the whole of what was communicated, and the
     * person is looking at a conversation they can type into.
     *
     * The long-lived credential is presented directly as a Bearer token rather than
     * exchanged for a session first. A session exists so a `WebView` can carry
     * something a cookie jar understands; this is one request from native code.
     */
    @Throws(CoreClientException::class)
    suspend fun hand(
        baseUrl: HttpUrl,
        credential: String,
        files: List<HandedFile>,
    ): Unit = withContext(Dispatchers.IO) {
        val multipart = MultipartBody.Builder()
            .setType(MultipartBody.FORM)
            .apply {
                files.forEach { file ->
                    addFormDataPart("file", file.name, file.body())
                }
            }
            .build()

        val request = Request.Builder()
            .url(endpoint(baseUrl, "api/in/file"))
            .post(multipart)
            .header("Authorization", "Bearer $credential")
            .build()
        send(request)
    }

    /**
     * Say something to a core — `POST /api/in/text`, the typed-input door.
     *
     * This is where a shared **link** goes, and the choice is deliberate: a URL is
     * something a person says, not an artifact they hand over. Filed through the file
     * channel it would be a few bytes on disk under a generated name, which the agent
     * would have to open to discover was a link; said, it is a line in the
     * conversation that reads exactly as it would if they had typed it.
     */
    @Throws(CoreClientException::class)
    suspend fun say(
        baseUrl: HttpUrl,
        credential: String,
        text: String,
    ): Unit = withContext(Dispatchers.IO) {
        val request = Request.Builder()
            .url(endpoint(baseUrl, "api/in/text"))
            .post(text.toRequestBody("text/plain; charset=utf-8".toMediaType()))
            .header("Authorization", "Bearer $credential")
            .build()
        send(request)
    }

    private fun send(request: Request) {
        // Uploading is not a four-second business; the health check's timeout is its
        // own and stays where it is.
        val client = http.newBuilder()
            .callTimeout(java.time.Duration.ofMinutes(30))
            .writeTimeout(java.time.Duration.ofMinutes(30))
            .build()

        val response = try {
            client.newCall(request).execute()
        } catch (e: Exception) {
            throw CoreClientException.RequestFailed(
                e.message ?: "The core could not be reached.",
            )
        }
        response.use {
            val detail = try {
                it.body.string().trim()
            } catch (_: Exception) {
                ""
            }
            // 207 is `/api/in/file` saying some parts landed and some did not.
            // Reading it as success is how a file that never landed would be
            // reported as sent.
            if (!it.isSuccessful || it.code == 207) {
                throw CoreClientException.Rejected(it.code, detail)
            }
        }
    }

    /**
     * `POST /api/access/request` — ask an agent to let this device in.
     *
     * Open at the core, because the caller is by definition a device with no way
     * in. What comes back is a code to show the person and a secret to poll with —
     * no access yet, and nothing worth storing.
     */
    @Throws(CoreClientException::class)
    suspend fun askToJoin(baseUrl: HttpUrl, label: String): JoinTicket =
        withContext(Dispatchers.IO) {
            val body = JSONObject().put("label", label.trim()).toString()
            val request = Request.Builder()
                .url(endpoint(baseUrl, "api/access/request"))
                .post(body.toRequestBody(jsonMediaType))
                .build()

            val response = try {
                http.newCall(request).execute()
            } catch (e: Exception) {
                throw CoreClientException.RequestFailed(
                    e.message ?: "That agent could not be reached.",
                )
            }

            response.use {
                val text = try {
                    it.body.string()
                } catch (_: Exception) {
                    ""
                }
                // A name nobody has claimed reaches the relay and stops there; an
                // address with no core awake behind it answers 503. To the person
                // typing, both mean the same thing: nothing is there under that name.
                if (it.code == 404 || it.code == 503) throw CoreClientException.NoSuchAgent
                if (it.code == 429) throw CoreClientException.TooManyWaiting
                if (!it.isSuccessful) {
                    throw CoreClientException.Rejected(it.code, text.trim())
                }
                try {
                    val json = JSONObject(text)
                    JoinTicket(
                        code = json.getString("code"),
                        secret = json.getString("secret"),
                    )
                } catch (_: Exception) {
                    throw CoreClientException.RequestFailed(
                        "The agent returned an unexpected answer.",
                    )
                }
            }
        }

    /**
     * `GET /api/access/request` — read where one request got to, presenting the
     * secret that came back with it.
     */
    @Throws(CoreClientException::class)
    suspend fun pollJoin(baseUrl: HttpUrl, secret: String): JoinState =
        withContext(Dispatchers.IO) {
            val request = Request.Builder()
                .url(endpoint(baseUrl, "api/access/request"))
                .get()
                .header("Authorization", "Bearer $secret")
                .build()
            val client = http.newBuilder()
                .callTimeout(java.time.Duration.ofSeconds(10))
                .build()

            val response = try {
                client.newCall(request).execute()
            } catch (e: Exception) {
                throw CoreClientException.RequestFailed(
                    e.message ?: "That agent could not be reached.",
                )
            }

            response.use {
                val text = try {
                    it.body.string()
                } catch (_: Exception) {
                    ""
                }
                if (!it.isSuccessful) throw CoreClientException.InvalidResponse
                val json = try {
                    JSONObject(text)
                } catch (_: Exception) {
                    throw CoreClientException.InvalidResponse
                }
                when (json.optString("state")) {
                    "approved" -> {
                        val credential = json.optString("credential").ifEmpty { null }
                            ?: throw CoreClientException.InvalidResponse
                        JoinState.Approved(credential)
                    }
                    "denied" -> JoinState.Denied
                    "expired" -> JoinState.Expired
                    else -> JoinState.Waiting
                }
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
     * Append a path to the core's base. A core is at the root of its own origin,
     * so this is ordinary joining — but a self-hosted one may sit behind
     * somebody's own reverse proxy at a path, and a base that carries one keeps
     * it.
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
