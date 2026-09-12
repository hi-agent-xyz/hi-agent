package com.xiaoyuanzhu.hiagent.android.core

/**
 * Where the last thing handed over got to.
 *
 * `count` is what the banner needs to write its sentence: "Sent to Ana" reads wrong
 * over three photos from the browser. It counts what the person will see land in the
 * conversation — one message per file, plus one for the words if there were any.
 */
sealed interface HandoffState {
    data class Sending(val count: Int) : HandoffState

    data class Sent(val coreLabel: String, val count: Int) : HandoffState

    data class Failed(val reason: String) : HandoffState
}
