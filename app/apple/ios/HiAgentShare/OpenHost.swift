import SwiftUI

/// Bringing Hi Agent forward from the share sheet.
///
/// **Three of the four ways to do this are dead, and one of them crashes.** This
/// list is written down because every one of them looks right until it is run on a
/// device, and nothing in the compiler or the console says otherwise:
///
/// - `UIApplication.shared.open` — does not compile under app-extension API rules.
/// - `NSExtensionContext.open(_:)` — documented Today-widget-only; from a share
///   extension the completion handler fires `success = false` and nothing happens.
/// - Responder-chain `openURL:` (the deprecated one-argument selector) — force-returns
///   NO since iOS 18, logging `BUG IN CLIENT OF UIKIT`. This is what this file used to
///   do, and it never once worked on a current device.
/// - Responder-chain `openURL:options:completionHandler:` — **crashes** inside UIKit's
///   KVC probe of the options dictionary.
///
/// What works is SwiftUI's own `OpenURLAction`, reached by instantiating
/// `EnvironmentValues()` directly rather than through a view's environment. Public
/// API, no runtime reflection, no deprecated selector.
///
/// **It must be a Universal Link.** A custom scheme (`hiagent://`) goes through the
/// machinery Apple has been closing off; an `https` URL this domain claims in its
/// `apple-app-site-association` is routed to the app by Associated Domains, which is
/// a different path entirely and the one still open. Hence `shareURL` below rather
/// than the app's own scheme.
///
/// Still best-effort, and still nothing depends on it: the drop is on disk before
/// this is called, and the app drains the queue whenever it next comes forward. What
/// this buys is landing the person *in the conversation*, which is the reason to
/// share something to your agent at all.
enum OpenHost {
    /// The apex, and not the core's own address.
    ///
    /// A Universal Link only works for a domain listed in the app's
    /// `associated-domains` entitlement, so this cannot be the person's own core —
    /// `ana.hi-agent.xyz` is a tunnel into their machine, and a self-hosted core is
    /// on a domain this app has never heard of. The apex is the one host the site
    /// itself serves, and the claim there is narrowed to this one path so that a
    /// shared view link never opens somebody else's app. See
    /// `backend/internal/server/applinks.go` in the hi-agent.xyz repo.
    static func shareURL(dropID: String) -> URL? {
        URL(string: "https://hi-agent.xyz/ios-share/\(dropID)")
    }

    /// Ask iOS to hand the URL to the app.
    ///
    /// No result: `OpenURLAction` has no completion handler, so whether the app
    /// actually came forward is not observable from here. Reporting it would be
    /// inventing a fact — and nothing needs it, because the fallback is the same
    /// either way.
    @MainActor
    static func open(_ url: URL) {
        EnvironmentValues().openURL(url)
    }
}
