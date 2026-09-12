import UIKit

/// Bringing Hi Agent forward from the share sheet.
///
/// **There is no sanctioned way for a share extension to open its own app, and this
/// file is the unsanctioned one.** `UIApplication.shared` does not compile under app
/// extension API restrictions; `NSExtensionContext.open(_:)` is documented for widgets
/// and does nothing here on most iOS versions. What works, and what most apps that do
/// this use, is that `UIApplication` is still in the responder chain and still
/// responds to the old `openURL:` selector — so the chain is walked until something
/// answers to it.
///
/// It is quarantined in its own file, with one call site, for a reason: **it is the
/// only part of sharing that could fail App Review, and nothing depends on it.**
/// Delete this file and the `OpenHost.open` line in `ShareViewController` and sharing
/// still works end to end — the drop is already on disk, and the app drains the queue
/// whenever it next comes forward. The person taps Hi Agent themselves instead of
/// arriving there. That is the whole difference, and it is why the open happens
/// *after* the enqueue rather than instead of it.
///
/// Android needs none of this: `ACTION_SEND` starts the activity, so the app is
/// simply already open.
enum OpenHost {
    @discardableResult
    static func open(_ url: URL, from responder: UIResponder) -> Bool {
        let selector = NSSelectorFromString("openURL:")
        var next: UIResponder? = responder
        while let current = next {
            if current.responds(to: selector) {
                _ = current.perform(selector, with: url)
                return true
            }
            next = current.next
        }
        return false
    }
}
