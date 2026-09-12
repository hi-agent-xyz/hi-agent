package com.xiaoyuanzhu.hiagent.android

import android.content.ContentResolver
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import com.xiaoyuanzhu.hiagent.android.core.HandedFile

/**
 * What arrived in an `ACTION_SEND`, as the two things a core has doors for.
 *
 * The split is the one decision here: **a link is words, everything else is a file.**
 * `EXTRA_TEXT` from the browser is a URL the person meant to say — sent to
 * `POST /api/in/text` it is a line in the conversation, and filed through the file
 * channel it would be a few bytes on disk under a generated name. `EXTRA_STREAM` is
 * an artifact, and goes to `POST /api/in/file`.
 *
 * Nothing here reads a byte. A `content://` URI is opened once, by OkHttp, when it is
 * ready to write the body — see [HandedFile].
 */
data class SharedIntent(val text: String?, val files: List<HandedFile>) {
    companion object {
        fun from(intent: Intent, resolver: ContentResolver): SharedIntent? {
            val uris = when (intent.action) {
                Intent.ACTION_SEND -> listOfNotNull(stream(intent))
                Intent.ACTION_SEND_MULTIPLE -> streams(intent)
                else -> return null
            }

            // The browser sends the page title in `EXTRA_SUBJECT` beside the URL.
            // It is not sent on: it is the page's sentence, not the person's, and
            // the agent can open the link and read a better one. Sharing means
            // handing over what was shared, not narrating it.
            val text = intent.getStringExtra(Intent.EXTRA_TEXT)?.takeIf { it.isNotBlank() }

            val files = uris.mapNotNull { uri -> handed(uri, resolver) }
            if (text == null && files.isEmpty()) return null
            return SharedIntent(text, files)
        }

        @Suppress("DEPRECATION")
        private fun stream(intent: Intent): Uri? =
            if (android.os.Build.VERSION.SDK_INT >= 33) {
                intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
            } else {
                intent.getParcelableExtra(Intent.EXTRA_STREAM)
            }

        @Suppress("DEPRECATION")
        private fun streams(intent: Intent): List<Uri> =
            if (android.os.Build.VERSION.SDK_INT >= 33) {
                intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
            } else {
                intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM)
            }.orEmpty()

        private fun handed(uri: Uri, resolver: ContentResolver): HandedFile? {
            val mime = resolver.getType(uri) ?: "application/octet-stream"
            var name: String? = null
            var length = -1L
            try {
                resolver.query(uri, null, null, null, null)?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        val nameColumn = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                        if (nameColumn >= 0 && !cursor.isNull(nameColumn)) {
                            name = cursor.getString(nameColumn)
                        }
                        val sizeColumn = cursor.getColumnIndex(OpenableColumns.SIZE)
                        if (sizeColumn >= 0 && !cursor.isNull(sizeColumn)) {
                            length = cursor.getLong(sizeColumn)
                        }
                    }
                }
            } catch (_: Exception) {
                // A provider that will not answer questions about its own file is
                // still a provider whose file can be read. Nothing here is required.
            }

            return HandedFile(
                name = filename(name, mime),
                mime = mime,
                length = length,
            ) {
                resolver.openInputStream(uri)
                    ?: throw java.io.IOException("Could not open what you shared.")
            }
        }

        /**
         * A name safe to put in a header, built here rather than taken verbatim.
         *
         * The display name comes from another app and lands in a
         * `Content-Disposition` line, so everything that is not a plain name
         * character goes — including the quote and the newline, which are the only
         * two that could do anything. A name that reduces to nothing falls back to a
         * timestamp and the mime's own extension: the core files the blob by its
         * extension, and `bin` is what it serves back when there is none.
         */
        private fun filename(suggested: String?, mime: String): String {
            val extension = MimeTypeMap.getSingleton()
                .getExtensionFromMimeType(mime.substringBefore(';').trim())
                ?: "bin"
            val clean = suggested.orEmpty()
                .filter { it.isLetterOrDigit() || it in "-_. " }
                .trim()
                .take(80)
            if (clean.isEmpty() || clean == ".") {
                val stamp = java.text.SimpleDateFormat(
                    "yyyyMMdd-HHmmss-SSS",
                    java.util.Locale.US,
                ).format(java.util.Date())
                return "shared-$stamp.$extension"
            }
            return if (clean.contains('.')) clean else "$clean.$extension"
        }
    }
}
