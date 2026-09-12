package com.xiaoyuanzhu.hiagent.android.core

import java.io.InputStream
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.RequestBody
import okio.BufferedSink
import okio.source
import org.json.JSONArray
import org.json.JSONObject

/** What the last `GET /healthz` found. Never persisted — a stored health is a lie. */
enum class HealthState {
    CHECKING,
    HERE,
    ASLEEP,
    UNREACHABLE,
    UNKNOWN,
    ;

    val title: String
        get() = when (this) {
            CHECKING -> "Checking"
            HERE -> "Available"
            ASLEEP -> "Asleep"
            UNREACHABLE -> "Unreachable"
            UNKNOWN -> "Unknown"
        }

    val isLive: Boolean get() = this == HERE
}

/**
 * One paired core as this device knows it. The label is local — the core has its
 * own name for this *device*, sent as `label` at exchange time, and the two are
 * not the same string.
 */
data class RosterEntry(
    val id: String,
    val label: String,
    val baseUrl: String,
    val addedAt: String,
    val attached: Boolean,
    val health: HealthState = HealthState.UNKNOWN,
) {
    /** `host:port/subpath` — what a person recognises, without the scheme noise. */
    val displayHost: String
        get() {
            val url = baseUrl.toHttpUrlOrNull() ?: return baseUrl
            val defaultPort = if (url.scheme == "https") 443 else 80
            val port = if (url.port == defaultPort) "" else ":${url.port}"
            val path = url.encodedPath.trim('/')
            return if (path.isEmpty()) "${url.host}$port" else "${url.host}$port/$path"
        }

    fun toJson(): JSONObject = JSONObject()
        .put("id", id)
        .put("label", label)
        .put("baseURL", baseUrl)
        .put("addedAt", addedAt)
        .put("attached", attached)

    companion object {
        fun fromJson(json: JSONObject) = RosterEntry(
            id = json.getString("id"),
            label = json.getString("label"),
            baseUrl = json.getString("baseURL"),
            addedAt = json.optString("addedAt"),
            attached = json.optBoolean("attached"),
            health = HealthState.UNKNOWN,
        )

        fun listFromJson(raw: String): List<RosterEntry> = buildList {
            val array = JSONArray(raw)
            for (i in 0 until array.length()) {
                add(fromJson(array.getJSONObject(i)))
            }
        }

        fun listToJson(entries: List<RosterEntry>): String =
            JSONArray().apply { entries.forEach { put(it.toJson()) } }.toString()
    }
}

/**
 * One file on its way to a core, as a thing that can be *opened* rather than as
 * bytes.
 *
 * What a share hands over is a `content://` URI belonging to another app, and the
 * only sane thing to do with it is stream it: [open] is called once, when OkHttp is
 * ready to write the body, and the bytes go from the other app's file to the socket
 * without this process ever holding them. That is what lets a two-gigabyte video be
 * shared from a phone with a few hundred megabytes of heap.
 *
 * [length] is `-1` when the provider will not say, which is normal for some content
 * providers; the upload is then chunked, which the core reads just the same.
 */
class HandedFile(
    val name: String,
    private val mime: String,
    private val length: Long,
    private val open: () -> InputStream,
) {
    fun body(): RequestBody = object : RequestBody() {
        override fun contentType() = mime.toMediaTypeOrNull()

        override fun contentLength() = length

        override fun writeTo(sink: BufferedSink) {
            open().use { stream -> sink.writeAll(stream.source()) }
        }
    }
}

/** An open session against one core: where it is, and the cookie that opens it. */
data class CoreSession(
    val entryId: String,
    val baseUrl: HttpUrl,
    val cookie: SessionCookie,
) {
    /** Renew a little before the cookie dies rather than after it has. */
    val needsRenewal: Boolean
        get() {
            val expiresAt = cookie.expiresAt ?: return false
            return expiresAt - System.currentTimeMillis() < 5 * 60 * 1000
        }
}
