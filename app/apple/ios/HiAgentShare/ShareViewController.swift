import Combine
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// Hi Agent in the iOS share sheet.
///
/// It copies what you shared into the App Group queue ([`HandedDrop`]) and offers
/// one button to go and talk about it. The upload itself happens in the app, not
/// here; the reasons are written on `HandedDrop`. The consequence worth knowing at
/// this end is that **finishing fast is correct** — this is a copy, not a send, so a
/// two-gigabyte video leaves the sheet about as quickly as a link does, and the
/// progress worth watching is in the conversation where the answer will appear too.
class ShareViewController: UIViewController {
    private let model = ShareModel()

    override func viewDidLoad() {
        super.viewDidLoad()

        model.openHost = { [weak self] url in
            Task { @MainActor in
                OpenHost.open(url)
                // **The gap is load-bearing.** Completing the extension request
                // tears its UI down, and that teardown races an open that has not
                // finished registering — it is silently cancelled and the app never
                // comes forward, with nothing anywhere to say why. So the open gets
                // a runloop tick first.
                //
                // The 50 ms is not a number chosen here: it is what a working
                // implementation of this same hand-off uses
                // (xiaoyuanzhu-com/my-life-db-apple), and this path cannot be
                // exercised anywhere but a real device, so borrowing a measured
                // value beats inventing one.
                try? await Task.sleep(for: .milliseconds(50))
                self?.finish()
            }
        }

        let hosted = UIHostingController(rootView: ShareView(model: model) { [weak self] in
            self?.finish()
        })
        addChild(hosted)
        hosted.view.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(hosted.view)
        NSLayoutConstraint.activate([
            hosted.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            hosted.view.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            hosted.view.topAnchor.constraint(equalTo: view.topAnchor),
            hosted.view.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        hosted.didMove(toParent: self)

        Task {
            await model.take(extensionContext?.inputItems as? [NSExtensionItem] ?? [])
        }
    }

    private func finish() {
        extensionContext?.completeRequest(returningItems: [], completionHandler: nil)
    }
}

/// What the share sheet is doing, and the one decision it offers.
@MainActor
final class ShareModel: ObservableObject {
    enum State: Equatable {
        case staging
        case ready(count: Int)
        case failed(reason: String)
    }

    @Published private(set) var state: State = .staging

    /// Set by the view controller: hand this URL to iOS, then tear the sheet down.
    var openHost: ((URL) -> Void)?

    private var dropID: String?

    /// Read every attachment and park the lot as one drop.
    func take(_ items: [NSExtensionItem]) async {
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

        guard !words.isEmpty || !files.isEmpty else {
            // An attachment type nobody could load, or an empty item. Nothing was
            // taken, so nothing is promised.
            state = .failed(reason: "There was nothing here Hi Agent could take.")
            return
        }

        let text = words.isEmpty ? nil : words.joined(separator: "\n")
        let count = files.count + (text == nil ? 0 : 1)

        // Off the main thread, because this is where the bytes actually move: a
        // video is copied a megabyte at a time, and the extension's main thread is
        // the one the system watches for a share sheet that has stopped responding.
        let staged: Result<HandedDrop?, Error> = await Task.detached(priority: .userInitiated) {
            do {
                return .success(try HandedDrop.enqueue(
                    // No note. What was shared is the whole of what was
                    // communicated, and the button below leads to a conversation
                    // the person can type into — a sentence written on their behalf
                    // here would be one they did not say, standing exactly where
                    // theirs is about to go.
                    note: nil,
                    text: text,
                    files: files
                ))
            } catch {
                return .failure(error)
            }
        }.value

        switch staged {
        case .success(let drop?):
            dropID = drop.id
            state = .ready(count: count)
        case .success(nil):
            state = .failed(reason: HandedDropError.noContainer.localizedDescription)
        case .failure(let error):
            state = .failed(reason: error.localizedDescription)
        }
    }

    /// Go to the conversation. Falls back to simply closing when there is no link
    /// to open — what was shared is already on disk, so the app will send it the
    /// next time it is opened either way.
    func openApp(then done: @escaping () -> Void) {
        guard let dropID, let url = OpenHost.shareURL(dropID: dropID), let openHost else {
            done()
            return
        }
        openHost(url)
    }
}

/// Turning one `NSItemProvider` into either words or a file.
///
/// The order of the checks is the whole content of this type. `public.file-url`
/// *conforms to* `public.url`, so a document shared from Files answers yes to both,
/// and asking about URLs first would file every attachment as a link to a path that
/// stops existing when this process does. Files are therefore decided first, and a
/// URL only means a link once that is ruled out.
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

    /// The bytes of this attachment, or `nil` when it is not a file.
    ///
    /// `loadFileRepresentation` writes a temporary file and hands over its URL,
    /// which is exactly what the queue wants: the copy into the group container is
    /// then the only time the bytes move, and neither end ever holds them.
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
    /// quote and the newline that are the only two characters that could do
    /// anything. A name reduced to nothing falls back to the timestamp, because the
    /// core files the blob by the extension and `bin` is what it serves back when it
    /// has none.
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
