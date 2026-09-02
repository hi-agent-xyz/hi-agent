import Foundation
import UIKit
import UniformTypeIdentifiers

/// "Come and see this", from a phone.
///
/// On a Mac the same gesture is a double-tap of the right ⌘: the app grabs the
/// screen itself and hands the PNG to the core on the `file` channel
/// (`body::gesture::glance`). iOS has no equivalent — **an app cannot photograph
/// another app's screen**, and no entitlement changes that. The one thing on the
/// system that can is the Shortcuts *Take Screenshot* action, and the Action Button
/// runs a shortcut over whatever app is in front without leaving it.
///
/// So the picture is taken by the OS and handed to us: the person's shortcut is
/// `Take Screenshot → Show My Screen`, and [`ShowScreenIntent`] is the second half.
/// This is the same mechanism/policy split the desktop shell follows — whoever holds
/// the screen grant takes the pixels, and the part that knows what the gesture *meant*
/// stays here.
enum ShowScreen {
    /// What the person is taken to have said as they pressed the button. It is a
    /// sentence rather than a flag because the rung that decides whether to look —
    /// Reaction — never opens a file: this line is the entire basis on which a PNG
    /// on the `file` channel is distinguishable from a document someone filed.
    ///
    /// Names the device (`UIDevice.model` is "iPhone" or "iPad") because unlike the
    /// desktop's screen, this one is a phone-shaped view of a single app, and that
    /// changes what the picture is evidence of.
    @MainActor
    static var note: String {
        "Here's my \(UIDevice.current.model) screen right now."
    }

    /// The name the bytes land under. Generated rather than taken from the shortcut:
    /// the extension is what the core files the blob by, and a name that came in over
    /// an intent is not something to interpolate into a MIME header.
    static func filename(for type: UTType?, at date: Date) -> String {
        let stamp = DateFormatter()
        stamp.locale = Locale(identifier: "en_US_POSIX")
        stamp.dateFormat = "yyyyMMdd-HHmmss"
        return "screen-\(stamp.string(from: date)).\(extensionFor(type))"
    }

    static func mime(for type: UTType?) -> String {
        type?.preferredMIMEType ?? "image/png"
    }

    /// A conservative extension for the image the shortcut handed over. Anything
    /// unrecognized is called a PNG because that is what *Take Screenshot* produces;
    /// a wrong-but-plausible extension is better than `bin`, which the core would
    /// serve back as opaque bytes the face cannot render.
    private static func extensionFor(_ type: UTType?) -> String {
        guard let type else {
            return "png"
        }
        if type.conforms(to: .jpeg) {
            return "jpg"
        }
        if type.conforms(to: .heic) || type.conforms(to: .heif) {
            return "heic"
        }
        if type.conforms(to: .webP) {
            return "webp"
        }
        return "png"
    }
}

/// One screen on its way to a core — kept whole so a send that failed can be tried
/// again from the banner rather than asking the person to make the gesture twice.
struct PendingScreen: Equatable {
    let data: Data
    let filename: String
    let mime: String
    let note: String
}

/// Where a screen got to. `nil` on `AppModel` means nothing has been shown this
/// launch; the banner is absent, not empty.
enum ShowScreenState: Equatable {
    case sending
    case sent(coreLabel: String)
    case failed(reason: String)
}
