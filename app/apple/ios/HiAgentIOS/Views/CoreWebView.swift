import Foundation
import SwiftUI
import WebKit

struct CoreWebView: UIViewRepresentable {
    let session: CoreSession

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeUIView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .default()
        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.allowsBackForwardNavigationGestures = true
        webView.navigationDelegate = context.coordinator
        context.coordinator.cookieValue = session.cookie.value
        installCookieAndLoad(webView)
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        let addressChanged = webView.url?.absoluteString != session.baseURL.absoluteString
        let cookieChanged = context.coordinator.cookieValue != session.cookie.value
        guard addressChanged || cookieChanged else {
            return
        }
        context.coordinator.cookieValue = session.cookie.value
        installCookieAndLoad(webView)
    }

    private func installCookieAndLoad(_ webView: WKWebView) {
        let cookieStore = webView.configuration.websiteDataStore.httpCookieStore
        cookieStore.setCookie(session.cookie) {
            DispatchQueue.main.async {
                webView.load(URLRequest(url: session.baseURL))
            }
        }
    }

    final class Coordinator: NSObject, WKNavigationDelegate {
        var cookieValue: String?
    }
}
