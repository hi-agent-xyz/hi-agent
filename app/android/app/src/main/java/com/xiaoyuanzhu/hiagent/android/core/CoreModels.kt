package com.xiaoyuanzhu.hiagent.android.core

import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
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
