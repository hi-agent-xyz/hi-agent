package com.xiaoyuanzhu.hiagent.android.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ExpandMore
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.SignalWifiOff
import androidx.compose.material.icons.rounded.WifiOff
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.pointer.pointerInput
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
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
 * The attached core, edge to edge. **The face gets the whole screen**, including
 * the strip under the status bar: it is a full-bleed surface that publishes
 * `--hi-safe-top` from `env(safe-area-inset-top)` and holds its own content
 * clear, so a native bar above it would subtract a device-height's worth of the
 * very thing the app exists to show — permanently, to say a name that does not
 * change.
 *
 * So the app's own chrome is transient. It shows itself while the core is being
 * opened, whenever the core is not answering, and for a moment after the face
 * first paints — then it leaves. A drag down from the top edge brings it back.
 */
@Composable
fun CoreStage(
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
    var chromeCalled by remember { mutableStateOf(true) }
    var chromeCallToken by remember { mutableIntStateOf(0) }
    var openToken by remember { mutableIntStateOf(0) }

    /**
     * The chrome is on screen when it has something to say, or when it was just
     * asked for. A ready face nobody has reached for keeps the whole screen.
     */
    val chromeShown = chromeCalled || webViewState != WebViewState.READY || !isConnected

    fun callChrome() {
        chromeCalled = true
        chromeCallToken += 1
    }

    // One countdown per call, keyed on the call token: an earlier dwell that is
    // still running is cancelled rather than hiding the chrome out from under
    // the hand that just asked for it.
    LaunchedEffect(chromeCallToken) {
        if (!chromeCalled) return@LaunchedEffect
        delay(2600)
        chromeCalled = false
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

    // Foregrounding re-checks the core, and re-opens if the session is close to
    // expiry or the last attempt failed.
    val lifecycleOwner = LocalLifecycleOwner.current
    androidx.compose.runtime.DisposableEffect(lifecycleOwner, entry.id) {
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
                modifier = Modifier
                    .fillMaxSize()
                    .alpha(if (webViewState == WebViewState.READY) 1f else 0f),
                onEvent = { event ->
                    when (event) {
                        is CoreWebViewEvent.Ready -> {
                            errorMessage = null
                            webViewState = WebViewState.READY
                            // The dwell starts when the face paints, not when the
                            // view appeared: opening a relayed core can take
                            // seconds, and a countdown spent behind the loading
                            // veil would hand over a screen whose chrome had
                            // already come and gone.
                            callChrome()
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
                title = "You're offline",
                message = "Hi Agent reconnects to ${entry.label} as soon as " +
                    "this device is back online.",
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

        // The reach-for-it strip, under the chrome in z-order so a visible
        // capsule takes its own taps. Live only while the chrome is away.
        if (!chromeShown) {
            ChromeReveal(
                onCall = ::callChrome,
                modifier = Modifier.align(Alignment.TopCenter),
            )
        }

        AnimatedVisibility(
            visible = chromeShown,
            enter = fadeIn() + slideInVertically { -it / 3 },
            exit = fadeOut() + slideOutVertically { -it / 3 },
            modifier = Modifier.align(Alignment.TopCenter),
        ) {
            StageChrome(
                entry = entry,
                onOpenRoster = onOpenRoster,
                onReload = {
                    callChrome()
                    if (webViewState == WebViewState.READY) reloadToken += 1 else openToken += 1
                },
            )
        }
    }
}

/**
 * The app's chrome over the face: which core you are talking to, whether it is
 * answering, and the two things you might want — switch, or reload.
 *
 * A pair of floating capsules rather than a bar, because it comes and goes: a
 * full-width bar with a rule under it reads as part of the screen's structure
 * and is jarring when it leaves, while a capsule reads as something handed to
 * you. Nothing below it moves when it appears.
 */
@Composable
private fun StageChrome(
    entry: RosterEntry,
    onOpenRoster: () -> Unit,
    onReload: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .statusBarsPadding()
            .padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Surface(
            shape = CircleShape,
            color = MaterialTheme.colorScheme.surface.copy(alpha = 0.92f),
            tonalElevation = 3.dp,
            shadowElevation = 6.dp,
            onClick = onOpenRoster,
            modifier = Modifier.semantics {
                contentDescription =
                    "${entry.label}, ${entry.health.title}. Switch core"
            },
        ) {
            Row(
                modifier = Modifier.padding(start = 8.dp, end = 12.dp, top = 8.dp, bottom = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                StatusDot(state = entry.health, diameter = 8.dp)
                Text(
                    text = entry.label,
                    style = MaterialTheme.typography.labelLarge,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                )
                Icon(
                    Icons.Rounded.ExpandMore,
                    contentDescription = null,
                    modifier = Modifier.size(16.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        Spacer(Modifier.weight(1f))

        Surface(
            shape = CircleShape,
            color = MaterialTheme.colorScheme.surface.copy(alpha = 0.92f),
            tonalElevation = 3.dp,
            shadowElevation = 6.dp,
        ) {
            IconButton(onClick = onReload, modifier = Modifier.size(36.dp)) {
                Icon(
                    Icons.Rounded.Refresh,
                    contentDescription = "Reload the face",
                    modifier = Modifier.size(18.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/**
 * How the chrome is called back once it has gone: a drag down from the top edge,
 * and a grabber saying there is something to drag.
 *
 * The strip lives in the status-bar inset, which is the one band of the screen
 * the face keeps clear of by construction (`--hi-safe-top`), so taking touches
 * here costs the view nothing.
 */
@Composable
private fun ChromeReveal(onCall: () -> Unit, modifier: Modifier = Modifier) {
    Box(
        modifier = modifier
            .fillMaxWidth()
            .statusBarsPadding()
            .height(44.dp)
            .pointerInput(Unit) {
                detectVerticalDragGestures { _, dragAmount ->
                    if (dragAmount > 0) onCall()
                }
            }
            .semantics { contentDescription = "Show which core this is" },
        contentAlignment = Alignment.TopCenter,
    ) {
        Box(
            Modifier
                .padding(top = 6.dp)
                .width(34.dp)
                .height(4.dp)
                .clip(RoundedCornerShape(2.dp))
                .background(MaterialTheme.colorScheme.onSurface.copy(alpha = 0.18f)),
        )
    }
}

/** What to tell the reader when opening a core did not work. */
private fun connectionMessage(error: Throwable, label: String): String = when {
    error is CoreClientException.Rejected && error.status == 401 ->
        "This device's credential was not accepted. Pair it with the core again."

    error is CredentialException ->
        "This device is no longer paired with $label. Pair it again."

    else -> error.message ?: "The core could not be reached."
}
