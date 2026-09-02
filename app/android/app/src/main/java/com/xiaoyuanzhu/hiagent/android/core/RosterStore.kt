package com.xiaoyuanzhu.hiagent.android.core

import android.content.Context

/**
 * The roster: small, local, non-secret metadata about which cores this device
 * knows. The secret half lives in [CredentialStore]; keeping them apart is why
 * a failed roster write can never cost a credential.
 */
class RosterStore(context: Context) {
    private val prefs = context.applicationContext
        .getSharedPreferences("hi.agent.roster", Context.MODE_PRIVATE)

    fun load(): List<RosterEntry> {
        val raw = prefs.getString(KEY, null) ?: return emptyList()
        return try {
            RosterEntry.listFromJson(raw)
        } catch (_: Exception) {
            prefs.edit().remove(KEY).apply()
            emptyList()
        }
    }

    fun save(entries: List<RosterEntry>) {
        try {
            prefs.edit().putString(KEY, RosterEntry.listToJson(entries)).apply()
        } catch (_: Exception) {
            // Deliberately swallowed. This holds local metadata only; a failed
            // write must not discard the in-memory selection or the credential.
        }
    }

    private companion object {
        const val KEY = "hi.agent.android.roster.v1"
    }
}
