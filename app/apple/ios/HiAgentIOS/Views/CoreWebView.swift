import Foundation
import SwiftUI
import WebKit

enum CoreWebViewEvent {
    case ready
    case sessionExpired
    case failed(Error)
}

struct CoreWebView: UIViewRepresentable {
    let session: CoreSession
    /// Bumped by the shell to ask for a reload. A face that has hung is
    /// otherwise a dead end: the session is valid, so nothing re-opens it.
    var reloadToken: Int = 0
    let onEvent: (CoreWebViewEvent) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(onEvent: onEvent, reloadToken: reloadToken)
    }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .default()
        configuration.userContentController.add(
            context.coordinator,
            name: Coordinator.sessionMessageName
        )
        configuration.userContentController.addUserScript(
            WKUserScript(
                source: Coordinator.sessionObserverScript,
                injectionTime: .atDocumentStart,
                forMainFrameOnly: false
            )
        )

        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.allowsBackForwardNavigationGestures = true
        // Let the canvas show through until the face paints, so opening a core
        // in dark appearance does not flash a white page.
        webView.isOpaque = false
        webView.backgroundColor = .clear
        webView.scrollView.backgroundColor = .clear
        webView.navigationDelegate = context.coordinator
        webView.uiDelegate = context.coordinator
        context.coordinator.install(session, in: webView)
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        context.coordinator.onEvent = onEvent
        context.coordinator.install(session, in: webView)
        context.coordinator.reloadIfRequested(reloadToken, in: webView)
    }

    static func dismantleUIView(_ webView: WKWebView, coordinator: Coordinator) {
        webView.configuration.userContentController.removeScriptMessageHandler(
            forName: Coordinator.sessionMessageName
        )
        webView.navigationDelegate = nil
        webView.uiDelegate = nil
    }

    final class Coordinator: NSObject, WKNavigationDelegate, WKScriptMessageHandler, WKUIDelegate {
        static let sessionMessageName = "hiAgentSession"
        static let sessionObserverScript = """
        (() => {
          if (window.__hiAgentSessionObserverInstalled) return;
          window.__hiAgentSessionObserverInstalled = true;
          const originalFetch = window.fetch.bind(window);
          window.fetch = async (...args) => {
            const response = await originalFetch(...args);
            if (response.status === 401) {
              window.webkit.messageHandlers.hiAgentSession.postMessage("unauthorized");
            }
            return response;
          };
        })();
        """

        var onEvent: (CoreWebViewEvent) -> Void

        private var session: CoreSession?
        private var installedCookieValue: String?
        private var renewalRequested = false
        private var reloadToken: Int

        init(onEvent: @escaping (CoreWebViewEvent) -> Void, reloadToken: Int = 0) {
            self.onEvent = onEvent
            self.reloadToken = reloadToken
        }

        func reloadIfRequested(_ token: Int, in webView: WKWebView) {
            guard token != reloadToken else {
                return
            }
            reloadToken = token
            renewalRequested = false
            webView.reload()
        }

        func install(_ nextSession: CoreSession, in webView: WKWebView) {
            let sessionChanged = session?.baseURL != nextSession.baseURL
                || installedCookieValue != nextSession.cookie.value
            session = nextSession
            guard sessionChanged else {
                return
            }

            installedCookieValue = nextSession.cookie.value
            renewalRequested = false
            webView.configuration.websiteDataStore.httpCookieStore.setCookie(nextSession.cookie) {
                DispatchQueue.main.async {
                    webView.load(URLRequest(url: nextSession.baseURL))
                }
            }
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            renewalRequested = false
            onEvent(.ready)
        }

        func webView(
            _ webView: WKWebView,
            didFailProvisionalNavigation navigation: WKNavigation!,
            withError error: Error
        ) {
            report(error)
        }

        func webView(
            _ webView: WKWebView,
            didFail navigation: WKNavigation!,
            withError error: Error
        ) {
            report(error)
        }

        func webView(
            _ webView: WKWebView,
            decidePolicyFor navigationResponse: WKNavigationResponse,
            decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void
        ) {
            if navigationResponse.isForMainFrame,
               let response = navigationResponse.response as? HTTPURLResponse,
               response.statusCode == 401 {
                requestSessionRenewal()
                decisionHandler(.cancel)
                return
            }
            decisionHandler(.allow)
        }

        func userContentController(
            _ userContentController: WKUserContentController,
            didReceive message: WKScriptMessage
        ) {
            guard message.name == Self.sessionMessageName,
                  message.body as? String == "unauthorized",
                  isTrusted(message.frameInfo.securityOrigin)
            else {
                return
            }
            requestSessionRenewal()
        }

        func webView(
            _ webView: WKWebView,
            requestMediaCapturePermissionFor origin: WKSecurityOrigin,
            initiatedByFrame frame: WKFrameInfo,
            type: WKMediaCaptureType,
            decisionHandler: @escaping (WKPermissionDecision) -> Void
        ) {
            decisionHandler(isTrusted(origin) ? .grant : .deny)
        }

        private func requestSessionRenewal() {
            guard !renewalRequested else {
                return
            }
            renewalRequested = true
            onEvent(.sessionExpired)
        }

        private func report(_ error: Error) {
            if (error as? URLError)?.code == .cancelled {
                return
            }
            onEvent(.failed(error))
        }

        private func isTrusted(_ origin: WKSecurityOrigin) -> Bool {
            guard let baseURL = session?.baseURL,
                  let scheme = baseURL.scheme?.lowercased(),
                  let host = baseURL.host?.lowercased()
            else {
                return false
            }

            let defaultPort = scheme == "https" ? 443 : 80
            let expectedPort = baseURL.port ?? defaultPort
            let actualPort = origin.port == 0 ? defaultPort : origin.port
            return origin.protocol.lowercased() == scheme
                && origin.host.lowercased() == host
                && actualPort == expectedPort
        }
    }
}
