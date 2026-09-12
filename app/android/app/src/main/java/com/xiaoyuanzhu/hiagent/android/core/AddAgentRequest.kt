package com.xiaoyuanzhu.hiagent.android.core

import android.net.Uri
import java.util.UUID

class AddAgentLinkException(message: String) : Exception(message)

/**
 * What opened the add screen, and what it should arrive holding.
 *
 * The `hiagent://pair?url=…&code=…` link it parses is **unchanged** — that is the
 * wire, minted by the core's `pairing_app_url()` and read by four other clients.
 * Only the words on this side moved.
 */
data class AddAgentRequest(
    val baseUrl: String,
    val code: String,
    val label: String,
    /** Set when the person picked "scan", so the sheet arrives with the camera up. */
    val opensScanner: Boolean = false,
    val id: String = UUID.randomUUID().toString(),
) {
    companion object {
        /** The ordinary way in: a name, and a person to approve it. */
        fun ask() = AddAgentRequest("", "", "")

        fun scan() = AddAgentRequest("", "", "", opensScanner = true)

        @Throws(AddAgentLinkException::class)
        fun fromUri(uri: Uri): AddAgentRequest {
            if (!uri.scheme.equals("hiagent", ignoreCase = true) ||
                !uri.host.equals("pair", ignoreCase = true)
            ) {
                throw AddAgentLinkException("This is not a Hi Agent link.")
            }

            val rawBaseUrl = singleValue(uri, "url")
                ?.takeIf { it.isNotBlank() }
                ?: throw AddAgentLinkException(
                    "That link does not contain an agent's address.",
                )
            val rawCode = singleValue(uri, "code")
                ?.takeIf { it.isNotBlank() }
                ?: throw AddAgentLinkException(
                    "That link does not contain a one-time code.",
                )

            val baseUrl = try {
                CoreClient.normalizeBaseUrl(rawBaseUrl).toString()
            } catch (e: CoreClientException) {
                throw AddAgentLinkException(e.message ?: "That address is not usable.")
            }

            return AddAgentRequest(
                baseUrl = baseUrl,
                code = rawCode.trim(),
                label = singleValue(uri, "label")?.trim().orEmpty(),
            )
        }

        /**
         * A repeated parameter is refused rather than resolved. `?code=a&code=b`
         * has no single right answer, and picking one would be guessing about a
         * link that is already malformed.
         */
        private fun singleValue(uri: Uri, name: String): String? {
            val values = try {
                uri.getQueryParameters(name)
            } catch (_: UnsupportedOperationException) {
                return null
            }
            return values.singleOrNull()
        }
    }
}
