package com.xiaoyuanzhu.hiagent.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.SignalWifiOff
import androidx.compose.material.icons.rounded.WifiOff
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.xiaoyuanzhu.hiagent.android.AppModel
import com.xiaoyuanzhu.hiagent.android.core.CoreClientException
import com.xiaoyuanzhu.hiagent.android.core.CoreSession
import com.xiaoyuanzhu.hiagent.android.core.CredentialException
import com.xiaoyuanzhu.hiagent.android.core.PairingRequest
import com.xiaoyuanzhu.hiagent.android.core.RosterEntry
import com.xiaoyuanzhu.hiagent.android.web.CoreWebView
import com.xiaoyuanzhu.hiagent.android.web.CoreWebViewEvent
import kotlinx.coroutines.delay

private enum class WebViewState { LOADING, READY, FAILED }

/**
 * The attached core on the television: the face, full screen, and nothing else
 * until something is asked for.
 *
 * **Back is the whole navigation, and it is a two-step ladder out of the app.**
 * With the face up and the chrome away, Back brings the chrome back and stops
 * there; with the chrome up, Back is not consumed and the system closes the
 * activity. So Back-Back leaves, and the step in between is a screen that says
 * which core this is and offers the two things worth doing to it. The chrome
 * summoned that way does not time out — a control that vanishes while you are
 * deciding would make leaving the app a matter of timing.
 *
 * **The face gets no say in Back yet.** Once it has a `tv` shape it will have its
 * own things to go back out of — an agent view holding focus, the conversation
 * standing open — and neither is reachable from here, because Android does not
 * deliver `KEYCODE_BACK` to a page and the shell cannot ask a WebView a
 * synchronous question. The seam that fixes it is the existing document-start
 * bridge, one method wider: the face reports how deep it is, the shell caches the
 * number and forwards a `hi:back` event instead of consuming, exactly as the
 * desktop shell already dispatches `hi:lifecycle`. Not built here; it belongs
 * with the `tv` shape, since until that exists the face has no depth to report.
 */
@Composable
fun TvCoreStage(
    model: AppModel,
    entry: RosterEntry,
    onOpenRoster: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val isConnected by model.isConnected.collectAsStateWithLifecycle()
    val credentialRevision by model.credentialRevision.collectAsStateWithLifecycle()

    var session by remember { mutableStateOf<CoreSession?>(null) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var isLoading by remember { mutableStateOf(false) }
    var webViewState by remember { mutableStateOf(WebViewState.LOADING) }
    var reloadToken by remember { mutableIntStateOf(0) }
    var openToken by remember { mutableIntStateOf(0) }

    /** Held open by Back, until Back or a choice closes it. */
    var pinned by remember { mutableStateOf(false) }

    /** The short greeting after the face first paints. Times out; Back's does not. */
    var greeting by remember { mutableStateOf(false) }

    val chromeShown = pinned || greeting || webViewState != WebViewState.READY || !isConnected

    BackHandler(enabled = !chromeShown) { pinned = true }

    LaunchedEffect(greeting) {
        if (!greeting) return@LaunchedEffect
        delay(2600)
        greeting = false
    }

    LaunchedEffect(entry.id, credentialRevision, openToken, isConnected) {
        if (!isConnected || isLoading) return@LaunchedEffect
        isLoading = true
        errorMessage = null
        webViewState = WebViewState.LOADING
        try {
            session = model.open(entry.id)
        } catch (e: Exception) {
            session = null
            errorMessage = connectionMessage(e, entry.label)
            webViewState = WebViewState.FAILED
        } finally {
            isLoading = false
        }
    }

    // A television is rarely backgrounded and never pocketed, but it is switched
    // to another input and back, which is the same lifecycle event and the same
    // stale session on the other side of it.
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, entry.id) {
        val observer = LifecycleEventObserver { _, event ->
            if (event != Lifecycle.Event.ON_RESUME) return@LifecycleEventObserver
            if (session?.needsRenewal == true || webViewState == WebViewState.FAILED) {
                openToken += 1
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    Box(modifier.hiCanvas()) {
        session?.let { open ->
            CoreWebView(
                session = open,
                reloadToken = reloadToken,
                focusOnAttach = true,
                modifier = Modifier
                    .fillMaxSize()
                    .alpha(if (webViewState == WebViewState.READY) 1f else 0f),
                onEvent = { event ->
                    when (event) {
                        is CoreWebViewEvent.Ready -> {
                            errorMessage = null
                            webViewState = WebViewState.READY
                            greeting = true
                        }

                        is CoreWebViewEvent.SessionExpired -> openToken += 1

                        is CoreWebViewEvent.Failed -> {
                            errorMessage = event.message
                            webViewState = WebViewState.FAILED
                        }
                    }
                },
            )
        }

        when {
            !isConnected -> StatusScreen(
                icon = Icons.Rounded.WifiOff,
                title = "This television is offline",
                message = "Hi Agent reconnects to ${entry.label} as soon as the " +
                    "network is back.",
            )

            webViewState == WebViewState.READY -> Unit

            isLoading || webViewState == WebViewState.LOADING -> LoadingVeil(entry.label)

            else -> StatusScreen(
                icon = Icons.Rounded.SignalWifiOff,
                title = "Can't reach this core",
                message = errorMessage ?: "Check the core address and try again.",
                primary = StatusAction("Try again") { openToken += 1 },
                secondary = StatusAction("Pair again") {
                    model.requestPairing(
                        PairingRequest(
                            baseUrl = entry.baseUrl,
                            code = "",
                            label = entry.label,
                        ),
                    )
                },
            )
        }

        AnimatedVisibility(
            visible = chromeShown,
            enter = fadeIn() + slideInVertically { -it / 3 },
            exit = fadeOut() + slideOutVertically { -it / 3 },
            modifier = Modifier.align(Alignment.TopCenter),
        ) {
            TvStageChrome(
                entry = entry,
                takeFocus = pinned,
                onOpenRoster = {
                    pinned = false
                    onOpenRoster()
                },
                onReload = {
                    pinned = false
                    if (webViewState == WebViewState.READY) reloadToken += 1 else openToken += 1
                },
            )
        }
    }
}

/**
 * Which core this is, and the two things worth doing to it.
 *
 * Focus moves to it only when Back asked for it. The same bar shown while a core
 * is opening must not steal focus from the face that is about to paint underneath
 * — and must not take the D-pad hostage during the seconds when there is nothing
 * here worth pressing.
 */
@Composable
private fun TvStageChrome(
    entry: RosterEntry,
    takeFocus: Boolean,
    onOpenRoster: () -> Unit,
    onReload: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val firstControl = remember { FocusRequester() }
    LaunchedEffect(takeFocus) {
        if (takeFocus) runCatching { firstControl.requestFocus() }
    }

    Row(
        modifier = modifier.fillMaxWidth().overscan(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        StatusDot(state = entry.health, diameter = 12.dp)
        TvButton(
            text = entry.label,
            onClick = onOpenRoster,
            focusRequester = firstControl,
        )
        Spacer(Modifier.weight(1f))
        TvButton(text = "Reload", leading = Icons.Rounded.Refresh, onClick = onReload)
    }
}

/** What to tell the reader when opening a core did not work. */
private fun connectionMessage(error: Throwable, label: String): String = when {
    error is CoreClientException.Rejected && error.status == 401 ->
        "This television's credential was not accepted. Pair it with the core again."

    error is CredentialException ->
        "This television is no longer paired with $label. Pair it again."

    else -> error.message ?: "The core could not be reached."
}
