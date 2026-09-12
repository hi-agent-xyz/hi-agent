import UIKit
import UniformTypeIdentifiers

/// Hi Agent in the iOS share sheet.
///
/// **It has no interface.** Every other share extension asks you to pick an album, a
/// recipient, a folder — this one has exactly one destination and nothing to ask, so
/// the sheet appears and leaves. What it does instead is copy what you shared into the
/// App Group queue ([`HandedDrop`]) and try to bring the app forward, because the
/// reason to share something to your agent is to talk about it.
///
/// The upload itself happens in the app, not here; the reasons are written on
/// `HandedDrop`. The consequence worth knowing at this end is that **finishing fast is
/// correct** — this is a copy, not a send, so a two-gigabyte video leaves the sheet as
/// quickly as a link does, and the progress the person watches is in the conversation
/// where the answer will also appear.
class ShareViewController: UIViewController {
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        Task {
            await hand()
        }
    }

    private func hand() async {
        let items = (extensionContext?.inputItems as? [NSExtensionItem]) ?? []
        var words: [String] = []
        var files: [HandedDrop.PendingFile] = []

        for provider in items.flatMap({ $0.attachments ?? [] }) {
            switch await Attachment.read(provider) {
            case .words(let text):
                words.append(text)
            case .file(let file):
                files.append(file)
            case .none:
                break
            }
        }

        // Nothing survived the read — an attachment type nobody could load, or an
        // empty item. Ending the request is all there is; a share sheet has no good
        // way to say "that didn't work" that is better than going away.
        if words.isEmpty && files.isEmpty {
            finish()
            return
        }

        // Off the main thread, because this is where the bytes actually move: a video
        // is copied a megabyte at a time and the extension's main thread is the one
        // the system watches for a share sheet that has stopped responding.
        let text = words.isEmpty ? nil : words.joined(separator: "\n")
        await Task.detached(priority: .userInitiated) {
            do {
                _ = try HandedDrop.enqueue(
                    // No note. What was shared is the whole of what was communicated,
                    // and the person is about to be looking at a conversation they can
                    // type into — a sentence written on their behalf here would be one
                    // they did not say, standing where theirs is about to go.
                    note: nil,
                    text: text,
                    files: files
                )
            } catch {
                NSLog("hi-agent share: could not queue: \(error.localizedDescription)")
            }
        }.value

        OpenHost.open(URL(string: "hiagent://shared")!, from: self)
        finish()
    }

    private func finish() {
        extensionContext?.completeRequest(returningItems: [], completionHandler: nil)
    }
}

/// Turning one `NSItemProvider` into either words or a file.
///
/// The order of the checks is the whole content of this type. `public.file-url`
/// *conforms to* `public.url`, so a document shared from Files answers yes to both and
/// asking about URLs first would file every attachment as a link to a path that stops
/// existing when this process does. Files are therefore decided first, and a URL only
/// means a link once that is ruled out.
enum Attachment {
    case words(String)
    case file(HandedDrop.PendingFile)

    static func read(_ provider: NSItemProvider) async -> Attachment? {
        if let file = await fileRepresentation(provider) {
            return .file(file)
        }
        if provider.hasItemConformingToTypeIdentifier(UTType.url.identifier),
           let url = await loadItem(provider, UTType.url) as? URL {
            return .words(url.absoluteString)
        }
        if provider.hasItemConformingToTypeIdentifier(UTType.text.identifier) {
            if let text = await loadItem(provider, UTType.text) as? String {
                return .words(text)
            }
        }
        return nil
    }

    /// The type to ask for the bytes of, or `nil` when this attachment is not a file.
    ///
    /// `loadFileRepresentation` writes a temporary file and hands over its URL, which
    /// is exactly what the queue wants: the copy into the group container is then the
    /// only time the bytes move, and neither end ever holds them.
    private static func fileRepresentation(_ provider: NSItemProvider) async -> HandedDrop.PendingFile? {
        let candidates = provider.registeredContentTypes.filter { type in
            // A plain URL or a string is not a file even though `public.item` would
            // happily produce one containing the link's text.
            !type.conforms(to: .url) && !type.conforms(to: .text) && type.conforms(to: .data)
        }
        guard let type = candidates.first else {
            return nil
        }

        let url: URL? = await withCheckedContinuation { continuation in
            _ = provider.loadFileRepresentation(forTypeIdentifier: type.identifier) { url, _ in
                guard let url else {
                    continuation.resume(returning: nil)
                    return
                }
                // The callback's URL is deleted the moment it returns, so the copy
                // has to happen inside it rather than after.
                let staged = FileManager.default.temporaryDirectory
                    .appendingPathComponent(UUID().uuidString)
                do {
                    try FileManager.default.copyItem(at: url, to: staged)
                    continuation.resume(returning: staged)
                } catch {
                    continuation.resume(returning: nil)
                }
            }
        }
        guard let url else {
            return nil
        }

        return HandedDrop.PendingFile(
            filename: filename(provider.suggestedName, type: type),
            mime: type.preferredMIMEType ?? "application/octet-stream",
            source: .url(url)
        )
    }

    private static func loadItem(_ provider: NSItemProvider, _ type: UTType) async -> Any? {
        await withCheckedContinuation { continuation in
            provider.loadItem(forTypeIdentifier: type.identifier) { item, _ in
                continuation.resume(returning: item)
            }
        }
    }

    /// A name safe to put in a header, built here rather than taken from the share.
    ///
    /// `suggestedName` comes from another app and lands in a `Content-Disposition`
    /// line, so everything that is not a plain name character goes — including the
    /// quote and the newline that are the only two characters that could do anything.
    /// A name reduced to nothing falls back to the timestamp, because the core files
    /// the blob by the extension and `bin` is what it serves back when it has none.
    private static func filename(_ suggested: String?, type: UTType) -> String {
        let ext = type.preferredFilenameExtension ?? "bin"
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-_. "))
        let stem = (suggested as NSString?)?.deletingPathExtension ?? ""
        let clean = String(stem.unicodeScalars.filter { allowed.contains($0) })
            .trimmingCharacters(in: .whitespaces)
            .prefix(80)

        if clean.isEmpty {
            let stamp = DateFormatter()
            stamp.locale = Locale(identifier: "en_US_POSIX")
            stamp.dateFormat = "yyyyMMdd-HHmmss-SSS"
            return "shared-\(stamp.string(from: Date())).\(ext)"
        }
        return "\(clean).\(ext)"
    }
}
