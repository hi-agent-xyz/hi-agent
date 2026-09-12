import Foundation

/// The App Group both targets meet in.
///
/// A share extension is a **separate process with its own container**, so nothing
/// this app already keeps — the roster in `UserDefaults`, the credentials in the
/// Keychain — is visible to it by default. The group is what makes one of those
/// shared; the credentials deliberately stay unshared, because of who does the
/// sending (see [`HandedDrop`]).
enum AppGroup {
    static let identifier = "group.com.xiaoyuanzhu.hiagent"

    /// Where drops wait. `nil` only if the entitlement is missing, which is a build
    /// mistake rather than a runtime condition — callers say so and give up rather
    /// than inventing a fallback directory the other process would never look in.
    static var container: URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: identifier)
    }

    static var queue: URL? {
        container?.appendingPathComponent("handed", isDirectory: true)
    }
}

/// One act of handing something over, parked on disk until it lands.
///
/// **The extension writes these; the app sends them.** That split is the whole design
/// here, and it is not the obvious one — a share extension could perfectly well do its
/// own HTTP. Four things fall out of doing it this way:
///
/// - The extension never touches a credential, so the Keychain does not have to be
///   shared between the two targets. The only entitlement either gains is the group.
/// - Nothing is racing the extension's lifetime. A share extension is killed the
///   moment its sheet goes away, which for a phone video over cellular is long before
///   the upload would finish; the app has an ordinary process lifetime and a screen to
///   report on.
/// - The queue *is* the retry buffer, and it outlives both processes. A send that
///   failed is still on disk after a crash, a reboot, or a week in a tunnel.
/// - The person ends up in the conversation, which is the point: they shared something
///   in order to talk about it (see `OpenHost`).
///
/// The cost is that delivery waits for the app to come forward. On Android none of
/// this applies — `ACTION_SEND` launches the activity itself — which is why that side
/// has no queue.
///
/// ## Why the bytes are already framed
///
/// Each file is stored as a **complete multipart body**, boundary and all, rather than
/// as the file plus a manifest the app would wrap at send time. The extension has to
/// copy the bytes no matter what (the item provider's URL is security-scoped and does
/// not survive the process), so it may as well copy them into the shape the wire
/// wants — one write, one read, and no second full-size copy on a phone that may be
/// nearly full. `URLSession.upload(fromFile:)` then streams it without ever holding a
/// 3 GB video in memory, which is also the reason the core's `/api/in/file` carries no
/// size limit at all.
///
/// One request per file, not one per drop: it keeps each body a single stored file,
/// and it means a drop of nine photos reports — and retries — the one that failed
/// rather than all nine.
struct HandedDrop {
    /// The drop's own directory, `<group>/handed/<uuid>/`.
    let directory: URL
    let manifest: Manifest

    struct Manifest: Codable {
        let createdAt: Date
        /// What the person effectively said. Empty for an ordinary share — a photo
        /// handed over from the share sheet is just a photo, and the line they are
        /// about to type in the conversation is the framing. Filled only by the
        /// screen gesture, which has no conversation to type into.
        let note: String?
        /// Words, when what was shared *is* words: a link, a selection, a note. Goes
        /// to `POST /api/in/text`, because a URL is something a person says, not an
        /// artifact they hand over — filed as bytes it would be a document nobody
        /// opens.
        let text: String?
        let boundary: String
        let parts: [Part]

        struct Part: Codable {
            /// Filename within the drop directory, e.g. `0.part`.
            let file: String
            /// The original name, for the banner. The wire's copy of it is inside
            /// the framed body.
            let name: String
        }
    }

    var isEmpty: Bool {
        manifest.text == nil && manifest.parts.isEmpty
    }

    /// How many things the person will see land in the conversation: the files, plus
    /// the words if there were any. One request each, and one message each.
    var itemCount: Int {
        manifest.parts.count + ((manifest.text?.isEmpty == false) ? 1 : 0)
    }

    // MARK: Writing

    /// Park a drop in the queue. Returns `nil` when there is no group container,
    /// which can only mean the entitlement did not ship.
    ///
    /// `files` are handed as a closure per item rather than as `Data` so a big video
    /// is copied through a buffer instead of being read whole: the caller opens the
    /// source, this opens the destination, and neither end holds the file.
    static func enqueue(
        note: String?,
        text: String?,
        files: [PendingFile]
    ) throws -> HandedDrop? {
        guard let queue = AppGroup.queue else {
            return nil
        }
        let directory = queue.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        let boundary = "hi-agent.\(UUID().uuidString)"
        var parts: [Manifest.Part] = []
        // The note rides the first file, which is where the core reads it: a `note`
        // part applies to the file parts that follow it and is spent on the first.
        var pendingNote = note
        for (index, file) in files.enumerated() {
            let name = "\(index).part"
            try write(
                file,
                to: directory.appendingPathComponent(name),
                boundary: boundary,
                note: pendingNote
            )
            pendingNote = nil
            parts.append(Manifest.Part(file: name, name: file.filename))
        }

        // A note with no file to ride would be lost, so it becomes the words. This is
        // the screen gesture's own case only if the screenshot failed to copy.
        let words = text ?? (files.isEmpty ? note : nil)
        let manifest = Manifest(
            createdAt: Date(),
            note: note,
            text: words,
            boundary: boundary,
            parts: parts
        )
        try JSONEncoder.drop.encode(manifest)
            .write(to: directory.appendingPathComponent(manifestName), options: .atomic)
        return HandedDrop(directory: directory, manifest: manifest)
    }

    /// One file on its way, as the multipart part it will become.
    struct PendingFile {
        let filename: String
        let mime: String
        /// Where the bytes are now. The extension gets a temporary URL from the item
        /// provider and hands it straight over; nothing is read until the copy.
        let source: Source

        enum Source {
            case url(URL)
            case data(Data)
        }
    }

    private static func write(
        _ file: PendingFile,
        to destination: URL,
        boundary: String,
        note: String?
    ) throws {
        FileManager.default.createFile(atPath: destination.path, contents: nil)
        guard let sink = FileHandle(forWritingAtPath: destination.path) else {
            throw HandedDropError.cannotWrite(destination.lastPathComponent)
        }
        defer { try? sink.close() }

        var head = ""
        if let note, !note.isEmpty {
            head += "--\(boundary)\r\n"
            head += "Content-Disposition: form-data; name=\"note\"\r\n\r\n"
            head += note
            head += "\r\n"
        }
        head += "--\(boundary)\r\n"
        // Neither field is ever the person's text: the caller generates the name from
        // the item's type and reads the mime from its UTI, so nothing can carry a
        // quote or a newline into a header.
        head += "Content-Disposition: form-data; name=\"file\"; filename=\"\(file.filename)\"\r\n"
        head += "Content-Type: \(file.mime)\r\n\r\n"
        try sink.write(contentsOf: Data(head.utf8))

        switch file.source {
        case .data(let data):
            try sink.write(contentsOf: data)
        case .url(let url):
            guard let source = FileHandle(forReadingAtPath: url.path) else {
                throw HandedDropError.cannotRead(url.lastPathComponent)
            }
            defer { try? source.close() }
            // 1 MiB at a time: the point of the whole arrangement is that no single
            // buffer is ever the size of what was shared.
            while let chunk = try source.read(upToCount: 1 << 20), !chunk.isEmpty {
                try sink.write(contentsOf: chunk)
            }
        }

        try sink.write(contentsOf: Data("\r\n--\(boundary)--\r\n".utf8))
    }

    // MARK: Reading

    private static let manifestName = "drop.json"

    /// Everything waiting, oldest first — the order they were handed over in.
    static func queued() -> [HandedDrop] {
        guard let queue = AppGroup.queue,
              let directories = try? FileManager.default.contentsOfDirectory(
                  at: queue,
                  includingPropertiesForKeys: nil
              )
        else {
            return []
        }
        return directories
            .compactMap { directory -> HandedDrop? in
                guard let data = try? Data(contentsOf: directory.appendingPathComponent(manifestName)),
                      let manifest = try? JSONDecoder.drop.decode(Manifest.self, from: data)
                else {
                    // A directory with no readable manifest is a write that died
                    // halfway. Nothing can be made of it and nothing will improve it.
                    try? FileManager.default.removeItem(at: directory)
                    return nil
                }
                return HandedDrop(directory: directory, manifest: manifest)
            }
            .sorted { $0.manifest.createdAt < $1.manifest.createdAt }
    }

    func url(for part: Manifest.Part) -> URL {
        directory.appendingPathComponent(part.file)
    }

    /// Drop it. Called once the whole thing has landed — never per part, so a
    /// half-sent drop is retried from the start rather than silently truncated.
    func discard() {
        try? FileManager.default.removeItem(at: directory)
    }
}

enum HandedDropError: LocalizedError {
    case cannotRead(String)
    case cannotWrite(String)
    case noContainer

    var errorDescription: String? {
        switch self {
        case .cannotRead(let name):
            return "Could not read \(name)."
        case .cannotWrite(let name):
            return "Could not save \(name) to send."
        case .noContainer:
            return "This build cannot pass shared files to Hi Agent."
        }
    }
}

extension JSONEncoder {
    static let drop: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        return encoder
    }()
}

extension JSONDecoder {
    static let drop: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }()
}
