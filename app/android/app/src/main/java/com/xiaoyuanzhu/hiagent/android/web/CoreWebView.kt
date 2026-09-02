package com.xiaoyuanzhu.hiagent.android.web

import android.annotation.SuppressLint
import android.content.Intent
import android.net.Uri
import android.webkit.CookieManager
import android.webkit.JavascriptInterface
import android.webkit.PermissionRequest
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import com.xiaoyuanzhu.hiagent.android.core.CoreSession

sealed interface CoreWebViewEvent {
    data object Ready : CoreWebViewEvent

    data object SessionExpired : CoreWebViewEvent

    data class Failed(val message: String) : CoreWebViewEvent
}

/**
 * The core's face, in the system WebView.
 *
 * Everything unusual in here is one of two things: an Android default that is
 * wrong for a full-screen web face, or a WebKit affordance Android does not have
 * and that has to be rebuilt out of the callbacks it does.
 */
@SuppressLint("SetJavaScriptEnabled")
@Composable
fun CoreWebView(
    session: CoreSession,
    reloadToken: Int,
    onEvent: (CoreWebViewEvent) -> Unit,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val state = remember { CoreWebViewState(context.applicationContext) }
    state.onEvent = onEvent

    AndroidView(
        modifier = modifier,
        factory = {
            WebView(context).apply {
                settings.javaScriptEnabled = true
                settings.domStorageEnabled = true
                settings.mediaPlaybackRequiresUserGesture = false
                settings.loadWithOverviewMode = true
                settings.useWideViewPort = true
                // The face is a fixed-width app surface, not a desktop page
                // being pinched; its own layout owns the zoom story.
                settings.setSupportZoom(false)
                settings.builtInZoomControls = false

                // `mediaPlaybackRequiresUserGesture` is Android's version of the
                // iOS `mediaTypesRequiringUserActionForPlayback` trap, and it
                // fails the same way. It defaults to true, and WebKit and
                // Chromium both hang the restriction on the page's AudioContext
                // — the graph the *microphone* runs through. The face builds
                // that context on load, where there is no gesture, so without
                // this line the camera works, the mic is silently dead, and the
                // agent's own voice never plays. See the matching note in
                // `CoreWebView.swift`.

                isVerticalScrollBarEnabled = false
                isHorizontalScrollBarEnabled = false
                // Let the canvas show through until the face paints, so opening
                // a core in dark appearance does not flash a white page.
                setBackgroundColor(android.graphics.Color.TRANSPARENT)

                webViewClient = state.webViewClient
                webChromeClient = state.webChromeClient

                CookieManager.getInstance().setAcceptCookie(true)
                CookieManager.getInstance().setAcceptThirdPartyCookies(this, false)

                state.installDocumentStartScript(this, session)
                state.install(session, this)
            }
        },
        update = { webView ->
            state.install(session, webView)
            state.reloadIfRequested(reloadToken, webView)
        },
        onRelease = { webView ->
            state.onEvent = {}
            webView.webChromeClient = null
            webView.stopLoading()
            webView.destroy()
        },
    )
}

private class CoreWebViewState(private val appContext: android.content.Context) {
    var onEvent: (CoreWebViewEvent) -> Unit = {}

    private var session: CoreSession? = null
    private var installedCookieValue: String? = null
    private var renewalRequested = false
    private var reloadToken = 0

    /**
     * Whether this navigation already reported an HTTP error. Android has no
     * `decidePolicyFor navigationResponse`, so a 401 cannot be cancelled the way
     * the iOS client cancels it — the page loads and `onPageFinished` fires
     * regardless. Remembering that the response was an error is what stops us
     * calling a rendered "unauthorized" body Ready and showing it to the reader.
     */
    private var navigationFailed = false

    fun install(nextSession: CoreSession, webView: WebView) {
        val changed = session?.baseUrl != nextSession.baseUrl ||
            installedCookieValue != nextSession.cookie.value
        session = nextSession
        if (!changed) return

        installedCookieValue = nextSession.cookie.value
        renewalRequested = false
        navigationFailed = false

        val manager = CookieManager.getInstance()
        // The raw `Set-Cookie` line goes over unmodified rather than being
        // rebuilt from its parts: the core owns the cookie's attributes — Path,
        // Max-Age, SameSite — and reconstructing them here would be this app
        // quietly having an opinion about a policy it does not set.
        manager.setCookie(nextSession.baseUrl.toString(), nextSession.cookie.setCookieHeader) {
            manager.flush()
            webView.post { webView.loadUrl(nextSession.baseUrl.toString()) }
        }
    }

    fun reloadIfRequested(token: Int, webView: WebView) {
        if (token == reloadToken) return
        reloadToken = token
        renewalRequested = false
        navigationFailed = false
        webView.reload()
    }

    /**
     * The `WKUserScript(injectionTime: .atDocumentStart)` equivalent. The face's
     * own `fetch` can meet a 401 long after the page loaded — a session that
     * expired while the phone was in a pocket — and that is invisible to every
     * navigation callback.
     *
     * `addDocumentStartJavaScript` is origin-scoped by the platform, so the
     * script only runs on the paired core. Where it is unavailable (WebView
     * older than 107) it is injected at `onPageStarted` instead, which is a hair
     * later but carries the same guarantee, because the origin is checked before
     * injecting.
     *
     * The bridge object itself is not origin-scoped — `addJavascriptInterface`
     * has no such parameter. That is tolerable because of what the bridge *is*:
     * one argument-less method that returns nothing and can only ask the shell
     * to re-exchange a credential the WebView has never seen. A page that calls
     * it uninvited costs one redundant `POST /api/session`.
     */
    fun installDocumentStartScript(webView: WebView, forSession: CoreSession) {
        webView.addJavascriptInterface(SessionBridge(::requestSessionRenewal), "hiAgentSession")
        if (!WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) return
        val base = forSession.baseUrl
        runCatching {
            WebViewCompat.addDocumentStartJavaScript(
                webView,
                SESSION_OBSERVER,
                setOf("${base.scheme}://${base.host}:${base.port}"),
            )
        }
    }

    private fun requestSessionRenewal() {
        if (renewalRequested) return
        renewalRequested = true
        onEvent(CoreWebViewEvent.SessionExpired)
    }

    /** Same rule as the iOS `isTrusted`: exact scheme, host, and port. */
    private fun isTrusted(origin: Uri?): Boolean {
        val baseUrl = session?.baseUrl ?: return false
        val scheme = origin?.scheme?.lowercase() ?: return false
        val host = origin.host?.lowercase() ?: return false
        val defaultPort = if (scheme == "https") 443 else 80
        val port = origin.port.takeIf { it != -1 } ?: defaultPort
        return scheme == baseUrl.scheme && host == baseUrl.host && port == baseUrl.port
    }

    val webViewClient = object : WebViewClient() {
        override fun onPageStarted(view: WebView, url: String?, favicon: android.graphics.Bitmap?) {
            navigationFailed = false
            if (WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) return
            if (isTrusted(url?.let(Uri::parse))) {
                view.evaluateJavascript(SESSION_OBSERVER, null)
            }
        }

        override fun onPageFinished(view: WebView, url: String?) {
            if (navigationFailed) return
            renewalRequested = false
            onEvent(CoreWebViewEvent.Ready)
        }

        /**
         * The nearest thing Android has to reading a navigation's status code.
         * It cannot cancel the load, but it can say what came back — which is
         * all the shell needs to re-exchange the credential instead of showing
         * the reader a raw "unauthorized" page.
         */
        override fun onReceivedHttpError(
            view: WebView,
            request: WebResourceRequest,
            errorResponse: WebResourceResponse,
        ) {
            if (!request.isForMainFrame) return
            navigationFailed = true
            if (errorResponse.statusCode == 401) {
                requestSessionRenewal()
            } else {
                onEvent(
                    CoreWebViewEvent.Failed(
                        "The core answered with HTTP ${errorResponse.statusCode}.",
                    ),
                )
            }
        }

        override fun onReceivedError(
            view: WebView,
            request: WebResourceRequest,
            error: WebResourceError,
        ) {
            if (!request.isForMainFrame) return
            navigationFailed = true
            onEvent(CoreWebViewEvent.Failed(describe(error.errorCode)))
        }

        /**
         * A link out of the core's own origin leaves for the browser.
         *
         * iOS does not need this rule — a `WKWebView` that wanders is still
         * visibly a web view. Here the face is the whole screen with no address
         * bar, so an off-origin page would render inside the app's own chrome
         * wearing its identity. The session cookie is host-scoped and does not
         * travel, so this is about what the reader is being shown, not about
         * what leaks.
         */
        override fun shouldOverrideUrlLoading(
            view: WebView,
            request: WebResourceRequest,
        ): Boolean {
            if (!request.isForMainFrame) return false
            val target = request.url
            if (isTrusted(target)) return false
            return runCatching {
                view.context.startActivity(
                    Intent(Intent.ACTION_VIEW, target).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
                true
            }.getOrDefault(true)
        }
    }

    val webChromeClient = object : WebChromeClient() {
        /**
         * Camera and microphone, granted only to the paired core's exact origin.
         *
         * The app's own runtime permissions are requested up front by the shell,
         * so by the time the face asks, the answer here is about origin alone.
         * A request this app does not hold the OS permission for is denied
         * rather than queued — the face retries, and a silently pending grant
         * would look like a hung camera.
         */
        override fun onPermissionRequest(request: PermissionRequest) {
            if (!isTrusted(request.origin)) {
                request.deny()
                return
            }
            val granted = request.resources.filter { resource ->
                when (resource) {
                    PermissionRequest.RESOURCE_VIDEO_CAPTURE ->
                        hasPermission(android.Manifest.permission.CAMERA)
                    PermissionRequest.RESOURCE_AUDIO_CAPTURE ->
                        hasPermission(android.Manifest.permission.RECORD_AUDIO)
                    else -> false
                }
            }
            if (granted.isEmpty()) request.deny() else request.grant(granted.toTypedArray())
        }
    }

    private fun hasPermission(permission: String): Boolean =
        androidx.core.content.ContextCompat.checkSelfPermission(appContext, permission) ==
            android.content.pm.PackageManager.PERMISSION_GRANTED

    private fun describe(errorCode: Int): String = when (errorCode) {
        WebViewClient.ERROR_HOST_LOOKUP -> "The core address could not be found."
        WebViewClient.ERROR_CONNECT, WebViewClient.ERROR_IO ->
            "The connection to the core was lost."
        WebViewClient.ERROR_TIMEOUT -> "The core took too long to respond."
        WebViewClient.ERROR_FAILED_SSL_HANDSHAKE ->
            "The core's secure connection could not be verified."
        else -> "The core could not be reached."
    }

    private companion object {
        val SESSION_OBSERVER = """
        (() => {
          if (window.__hiAgentSessionObserverInstalled) return;
          window.__hiAgentSessionObserverInstalled = true;
          const originalFetch = window.fetch.bind(window);
          window.fetch = async (...args) => {
            const response = await originalFetch(...args);
            if (response.status === 401) {
              window.hiAgentSession.unauthorized();
            }
            return response;
          };
        })();
        """.trimIndent()
    }
}

/**
 * The JS side's only way to reach Kotlin. One method, no arguments, no return
 * value — there is nothing here for a page to read, and the credential is not
 * in this process's WebView at all.
 */
class SessionBridge(private val onUnauthorized: () -> Unit) {
    @JavascriptInterface
    fun unauthorized() {
        onUnauthorized()
    }
}
