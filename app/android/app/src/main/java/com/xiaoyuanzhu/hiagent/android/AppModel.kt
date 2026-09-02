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
import com.xiaoyuanzhu.hiagent.android.core.HealthState
import com.xiaoyuanzhu.hiagent.android.core.NetworkMonitor
import com.xiaoyuanzhu.hiagent.android.core.PairingLinkException
import com.xiaoyuanzhu.hiagent.android.core.PairingRequest
import com.xiaoyuanzhu.hiagent.android.core.RosterEntry
import com.xiaoyuanzhu.hiagent.android.core.RosterStore
import java.time.Instant
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.stateIn
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

    private val _pairingRequest = MutableStateFlow<PairingRequest?>(null)
    val pairingRequest: StateFlow<PairingRequest?> = _pairingRequest.asStateFlow()

    private val _pairingLinkError = MutableStateFlow<String?>(null)
    val pairingLinkError: StateFlow<String?> = _pairingLinkError.asStateFlow()

    /**
     * Bumped whenever a credential is written. The stage restarts its open on a
     * change, so re-pairing a core that was failing recovers without the person
     * having to find a Reload.
     */
    private val _credentialRevision = MutableStateFlow(0)
    val credentialRevision: StateFlow<Int> = _credentialRevision.asStateFlow()

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

    // MARK: Pairing

    fun requestPairing(request: PairingRequest?) {
        _pairingRequest.value = request
    }

    fun clearPairingLinkError() {
        _pairingLinkError.value = null
    }

    fun handleIncomingUri(uri: Uri) {
        try {
            _pairingRequest.value = PairingRequest.fromUri(uri)
            _pairingLinkError.value = null
        } catch (e: PairingLinkException) {
            _pairingLinkError.value = e.message
        }
    }

    /**
     * Exchange a pairing code for a credential and remember the core.
     *
     * `label` on the wire is what the *core* will call this device; the roster
     * label is what this device calls the core. They are different strings for
     * different readers, which is why only one of them is sent.
     */
    suspend fun pair(rawBaseUrl: String, rawCode: String, rawLabel: String) {
        val baseUrl = CoreClient.normalizeBaseUrl(rawBaseUrl)
        val code = rawCode.trim()
        if (code.isEmpty()) {
            throw CoreClientException.RequestFailed("Enter the pairing code from the core.")
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

    /**
     * Open a session against one core. The stored credential goes to
     * `POST /api/session` and only the exchanged cookie comes back out — the
     * credential itself never leaves this class.
     */
    suspend fun open(id: String): CoreSession {
        val entry = entry(id) ?: throw CoreClientException.InvalidAddress(
            "This core is no longer in the roster.",
        )
        val baseUrl = entry.baseUrl.toHttpUrlOrNull()
            ?: throw CoreClientException.InvalidAddress(
                "This core's address is no longer usable.",
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
        val url = baseUrl.toHttpUrlOrNull() ?: return "Core"
        val path = url.encodedPath.trim('/')
        return if (path.isEmpty()) url.host else "${url.host}/$path"
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
