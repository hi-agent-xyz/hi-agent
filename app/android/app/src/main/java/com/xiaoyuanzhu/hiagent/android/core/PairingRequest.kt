package com.xiaoyuanzhu.hiagent.android.core

import android.net.Uri
import java.util.UUID

class PairingLinkException(message: String) : Exception(message)

/**
 * A `hiagent://pair?url=…&code=…` link, from the QR the core's Reach view draws
 * or from a link handed to the app by anything else on the handset.
 *
 * The core builds this in `pairing_app_url()` and knows nothing about which
 * platform will read it, so nothing here is Android-specific.
 */
data class PairingRequest(
    val baseUrl: String,
    val code: String,
    val label: String,
    /** Scanning is the intended path, so the sheet can arrive with the camera up. */
    val opensScanner: Boolean = false,
    val id: String = UUID.randomUUID().toString(),
) {
    companion object {
        fun manual() = PairingRequest("", "", "")

        fun scan() = PairingRequest("", "", "", opensScanner = true)

        @Throws(PairingLinkException::class)
        fun fromUri(uri: Uri): PairingRequest {
            if (!uri.scheme.equals("hiagent", ignoreCase = true) ||
                !uri.host.equals("pair", ignoreCase = true)
            ) {
                throw PairingLinkException("This is not a Hi Agent pairing link.")
            }

            val rawBaseUrl = singleValue(uri, "url")
                ?.takeIf { it.isNotBlank() }
                ?: throw PairingLinkException(
                    "The pairing link does not contain a core address.",
                )
            val rawCode = singleValue(uri, "code")
                ?.takeIf { it.isNotBlank() }
                ?: throw PairingLinkException(
                    "The pairing link does not contain a pairing code.",
                )

            val baseUrl = try {
                CoreClient.normalizeBaseUrl(rawBaseUrl).toString()
            } catch (e: CoreClientException) {
                throw PairingLinkException(e.message ?: "The core address is not usable.")
            }

            return PairingRequest(
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
