package com.xiaoyuanzhu.hiagent.android

import android.app.Application
import android.net.Uri
import android.os.Build
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.xiaoyuanzhu.hiagent.android.core.CoreClient
import com.xiaoyuanzhu.hiagent.android.core.CoreClientException
import com.xiaoyuanzhu.hiagent.android.core.CoreSession
import com.xiaoyuanzhu.hiagent.android.core.CredentialStore
import com.xiaoyuanzhu.hiagent.android.core.HandedFile
import com.xiaoyuanzhu.hiagent.android.core.HandoffState
import com.xiaoyuanzhu.hiagent.android.core.HealthState
import com.xiaoyuanzhu.hiagent.android.core.JoinState
import com.xiaoyuanzhu.hiagent.android.core.NetworkMonitor
import com.xiaoyuanzhu.hiagent.android.core.AddAgentLinkException
import com.xiaoyuanzhu.hiagent.android.core.AddAgentRequest
import com.xiaoyuanzhu.hiagent.android.core.RosterEntry
import com.xiaoyuanzhu.hiagent.android.core.RosterStore
import java.time.Instant
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull

/**
 * All client state the shell needs: which cores this device knows, which one is
 * attached, and how to open one. The core owns identity, credential issuance and
 * revocation, memory, cognition, and channel behaviour — none of that is here.
 */
class AppModel(application: Application) : AndroidViewModel(application) {
    private val roster = RosterStore(application)
    private val credentials = CredentialStore(application)
    private val network = NetworkMonitor(application)

    private val _entries = MutableStateFlow<List<RosterEntry>>(emptyList())
    val entries: StateFlow<List<RosterEntry>> = _entries.asStateFlow()

    private val _selectedId = MutableStateFlow<String?>(null)
    val selectedId: StateFlow<String?> = _selectedId.asStateFlow()

    private val _isRefreshing = MutableStateFlow(false)
    val isRefreshing: StateFlow<Boolean> = _isRefreshing.asStateFlow()

    private val _addRequest = MutableStateFlow<AddAgentRequest?>(null)
    val addRequest: StateFlow<AddAgentRequest?> = _addRequest.asStateFlow()

    private val _addLinkError = MutableStateFlow<String?>(null)
    val addLinkError: StateFlow<String?> = _addLinkError.asStateFlow()

    /**
     * Bumped whenever a credential is written. The stage restarts its open on a
     * change, so adding an agent that was failing recovers without the person
     * having to find a Reload.
     */
    private val _credentialRevision = MutableStateFlow(0)
    val credentialRevision: StateFlow<Int> = _credentialRevision.asStateFlow()

    /**
     * Where the last thing handed over got to, or null if nothing has been this
     * launch — the banner is then absent, not empty. A share arrives while the face
     * is still painting, so without this the successful case and the "not paired with
     * anything" case look identical: the app opened, and nothing visibly happened.
     */
    private val _handoff = MutableStateFlow<HandoffState?>(null)
    val handoff: StateFlow<HandoffState?> = _handoff.asStateFlow()

    val isConnected: StateFlow<Boolean> = network.isConnected
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), true)

    init {
        val loaded = roster.load()
        _entries.value = loaded
        _selectedId.value = loaded.firstOrNull { it.attached }?.id
    }

    fun entry(id: String): RosterEntry? = _entries.value.firstOrNull { it.id == id }

    val current: RosterEntry?
        get() = _selectedId.value?.let { entry(it) }
            ?: _entries.value.firstOrNull { it.attached }
            ?: _entries.value.firstOrNull()

    // MARK: Adding an agent

    fun requestAdd(request: AddAgentRequest?) {
        _addRequest.value = request
    }

    fun clearAddLinkError() {
        _addLinkError.value = null
    }

    fun handleIncomingUri(uri: Uri) {
        try {
            _addRequest.value = AddAgentRequest.fromUri(uri)
            _addLinkError.value = null
        } catch (e: AddAgentLinkException) {
            _addLinkError.value = e.message
        }
    }

    /**
     * Ask an agent, by name, to let this device in.
     *
     * Nothing is stored by this call: what comes back is a code to show and a
     * secret to wait on. The roster only grows once [join] lands.
     */
    suspend fun askToJoin(name: String): JoinInvitation {
        val baseUrl = CoreClient.addressForName(name)
        val ticket = CoreClient.askToJoin(baseUrl, deviceName())
        return JoinInvitation(
            baseUrl = baseUrl.toString(),
            label = agentLabel(name, baseUrl.toString()),
            code = ticket.code,
            secret = ticket.secret,
        )
    }

    /**
     * Wait out one invitation, and take the credential the moment it is approved.
     *
     * Polls until answered or cancelled — the caller's coroutine is what ends it,
     * so leaving the screen stops the wait. Past the request's own ten-minute life
     * the agent answers `expired` and this throws, which is the same clock rather
     * than a second one kept here.
     */
    suspend fun join(invitation: JoinInvitation) {
        val baseUrl = CoreClient.normalizeBaseUrl(invitation.baseUrl)
        while (true) {
            when (val state = CoreClient.pollJoin(baseUrl, invitation.secret)) {
                is JoinState.Approved -> {
                    // From here it is the ordinary add: the credential goes to the
                    // Keystore, the agent joins the roster, and the stage opens it.
                    add(invitation.baseUrl, state.credential, invitation.label)
                    return
                }
                JoinState.Denied ->
                    throw CoreClientException.RequestFailed("That was turned down.")
                JoinState.Expired ->
                    throw CoreClientException.RequestFailed(
                        "Nobody answered in time. Ask again.",
                    )
                JoinState.Waiting -> delay(2_000)
            }
        }
    }

    /**
     * Exchange a one-time code — or a credential a [join] just claimed — and
     * remember the agent.
     *
     * `label` on the wire is what the *agent* will call this device; the roster
     * label is what this device calls the agent. They are different strings for
     * different readers, which is why only one of them is sent.
     */
    suspend fun add(rawBaseUrl: String, rawCode: String, rawLabel: String) {
        val baseUrl = CoreClient.normalizeBaseUrl(rawBaseUrl)
        val code = rawCode.trim()
        if (code.isEmpty()) {
            throw CoreClientException.RequestFailed(
                "Enter the one-time code the agent is showing.",
            )
        }
        val coreLabel = rawLabel.trim().ifEmpty { defaultCoreLabel(baseUrl.toString()) }

        val (exchange, _) = CoreClient.exchange(
            baseUrl = baseUrl,
            presented = code,
            label = deviceName(),
        )
        // Null means "keep what you have" — the core says that when a credential,
        // not a pairing code, was presented.
        val credential = exchange.credential ?: code
        credentials.save(credential, CredentialStore.account(exchange.id))
        _credentialRevision.value += 1

        val wasAttached = _entries.value.firstOrNull { it.id == exchange.id }?.attached
            ?: _entries.value.isEmpty()
        val entry = RosterEntry(
            id = exchange.id,
            label = coreLabel,
            baseUrl = baseUrl.toString(),
            addedAt = Instant.now().toString(),
            attached = wasAttached,
        )
        _entries.value = _entries.value.filterNot { it.id == entry.id } + entry
        if (entry.attached) {
            _selectedId.value = entry.id
        }
        persist()
        refresh(entry.id)
    }

    // MARK: Roster

    fun attach(id: String) {
        if (_entries.value.none { it.id == id }) return
        _entries.value = _entries.value.map { it.copy(attached = it.id == id) }
        _selectedId.value = id
        persist()
    }

    fun forget(id: String) {
        val removedWasAttached = _entries.value.firstOrNull { it.id == id }?.attached == true
        credentials.delete(CredentialStore.account(id))
        var remaining = _entries.value.filterNot { it.id == id }
        if (removedWasAttached && remaining.isNotEmpty()) {
            remaining = remaining.mapIndexed { index, entry ->
                entry.copy(attached = index == 0)
            }
        }
        _entries.value = remaining
        if (_selectedId.value == id || _selectedId.value == null) {
            _selectedId.value = remaining.firstOrNull { it.attached }?.id
                ?: remaining.firstOrNull()?.id
        }
        persist()
    }

    fun refreshAll() {
        viewModelScope.launch {
            _isRefreshing.value = true
            try {
                _entries.value.forEach { refresh(it.id) }
            } finally {
                _isRefreshing.value = false
            }
        }
    }

    suspend fun refresh(entryId: String) {
        val baseUrl = entry(entryId)?.baseUrl?.toHttpUrlOrNull() ?: return
        setHealth(entryId, HealthState.CHECKING)
        setHealth(entryId, CoreClient.health(baseUrl))
    }

    // MARK: Handing things over

    /**
     * Send what the person just shared to the attached core.
     *
     * **Straight out over the wire, with no queue** — unlike iOS, where a share
     * extension is a separate short-lived process that cannot reach the credential or
     * outlive its sheet. `ACTION_SEND` starts this activity, so by the time this runs
     * the app is open, the roster is loaded, the keychain is readable and there is a
     * conversation on screen to report into. There is nothing to park.
     *
     * Words go first when there are both: a link shared with a picture of it lands as
     * the sentence and then the artifact, which is the order the person did it in.
     *
     * Reports rather than throws: the caller is an intent, which has nowhere to put an
     * error.
     *
     * **A send that outlives the screen is not handled, and it is the known gap here.**
     * The upload runs in `viewModelScope` and reads through a `content://` grant that
     * belongs to the activity, so walking away from a large video mid-upload can lose
     * both: the scope is cancelled with the activity, and the grant is revoked with
     * the task. Small things — the photos and links that are almost all of sharing —
     * land long before that matters. Closing it means a foreground service holding the
     * upload and a copy of the bytes taken while the grant is live, which is the
     * Android shape of what iOS does with its on-disk queue.
     */
    fun share(text: String?, files: List<HandedFile>) {
        if (text.isNullOrBlank() && files.isEmpty()) return

        val entry = current
        if (entry == null) {
            _handoff.value = HandoffState.Failed(
                "This device hasn't been added to an agent yet, so there's nobody to hand it to.",
            )
            return
        }
        val baseUrl = entry.baseUrl.toHttpUrlOrNull()
        if (baseUrl == null) {
            _handoff.value = HandoffState.Failed("This agent's address is no longer usable.")
            return
        }

        val count = files.size + if (text.isNullOrBlank()) 0 else 1
        _handoff.value = HandoffState.Sending(count)
        viewModelScope.launch {
            try {
                val credential = credentials.read(CredentialStore.account(entry.id))
                if (!text.isNullOrBlank()) {
                    CoreClient.say(baseUrl, credential, text.trim())
                }
                if (files.isNotEmpty()) {
                    CoreClient.hand(baseUrl, credential, files)
                }
                _handoff.value = HandoffState.Sent(entry.label, count)
            } catch (e: Exception) {
                _handoff.value = HandoffState.Failed(
                    e.message ?: "That could not be sent.",
                )
            }
        }
    }

    fun dismissHandoff() {
        _handoff.value = null
    }

    /**
     * Open a session against one core. The stored credential goes to
     * `POST /api/session` and only the exchanged cookie comes back out — the
     * credential itself never leaves this class.
     */
    suspend fun open(id: String): CoreSession {
        val entry = entry(id) ?: throw CoreClientException.InvalidAddress(
            "This agent is no longer in the roster.",
        )
        val baseUrl = entry.baseUrl.toHttpUrlOrNull()
            ?: throw CoreClientException.InvalidAddress(
                "This agent's address is no longer usable.",
            )
        val credential = credentials.read(CredentialStore.account(id))
        val (_, cookie) = CoreClient.exchange(baseUrl, credential, entry.label)
        setHealth(id, HealthState.HERE)
        return CoreSession(entryId = id, baseUrl = baseUrl, cookie = cookie)
    }

    private fun setHealth(entryId: String, state: HealthState) {
        _entries.value = _entries.value.map {
            if (it.id == entryId) it.copy(health = state) else it
        }
    }

    private fun persist() = roster.save(_entries.value)

    private fun defaultCoreLabel(baseUrl: String): String {
        val url = baseUrl.toHttpUrlOrNull() ?: return "Agent"
        val path = url.encodedPath.trim('/')
        return if (path.isEmpty()) url.host else "${url.host}/$path"
    }

    /**
     * What to call an agent added by name. The name itself when that is what was
     * typed — "iloahz" reads better in the roster than "iloahz.hi-agent.xyz" — and
     * the host when somebody pasted a whole address.
     */
    private fun agentLabel(name: String, baseUrl: String): String {
        val bare = name.trim().lowercase().removePrefix("@")
        return if (bare.isNotEmpty() && !bare.contains('.') && !bare.contains('/')) {
            bare
        } else {
            defaultCoreLabel(baseUrl)
        }
    }

    /**
     * What the core will list this device as. Android has no
     * `UIDevice.current.name`, and since Android 10 the user-set device name is
     * not readable without permissions the app has no other use for — so this is
     * the marketing name, which is what a person recognises in a device list
     * anyway.
     */
    private fun deviceName(): String {
        val manufacturer = Build.MANUFACTURER.orEmpty().replaceFirstChar { it.uppercase() }
        val model = Build.MODEL.orEmpty()
        return when {
            model.isEmpty() -> manufacturer.ifEmpty { "Android device" }
            model.startsWith(manufacturer, ignoreCase = true) -> model
            manufacturer.isEmpty() -> model
            else -> "$manufacturer $model"
        }
    }
}

/** One outstanding "let me in", held only while the add screen is open. */
data class JoinInvitation(
    val baseUrl: String,
    val label: String,
    /**
     * Shown here and on the agent's Reach view, for the two to be compared. It
     * authorizes nothing.
     */
    val code: String,
    val secret: String,
)
