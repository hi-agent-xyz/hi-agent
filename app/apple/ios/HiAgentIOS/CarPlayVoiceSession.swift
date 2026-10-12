import AVFoundation
import Combine
import Foundation

@MainActor
final class CarPlayVoiceSession: ObservableObject {
    static let shared = CarPlayVoiceSession()
    enum State: String { case idle, connecting, listening, speaking, failed }
    @Published private(set) var state: State = .idle
    @Published private(set) var message = ""
    @Published private(set) var entryID: String?
    var ownsAudio: Bool { state == .connecting || state == .listening || state == .speaking }

    private var generation = UUID()
    private var tasks: [Task<Void, Never>] = []
    private var transport: URLSession?
    private var socket: URLSessionWebSocketTask?
    private var engine: AVAudioEngine?
    private var player: CarPlayAudioPlayer?
    private var frames: AsyncStream<Data>.Continuation?
    private var observers: [NSObjectProtocol] = []
    private var lastFrame = Date()
    private var speechRevision = 0

    private init() {
        let center = NotificationCenter.default
        for name in [AVAudioSession.interruptionNotification, AVAudioSession.routeChangeNotification,
                     AVAudioSession.mediaServicesWereResetNotification] {
            observers.append(center.addObserver(forName: name, object: nil, queue: .main) { [weak self] note in
                Task { @MainActor in
                    guard let self, self.ownsAudio else { return }
                    if name == AVAudioSession.interruptionNotification,
                       (note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt) != AVAudioSession.InterruptionType.began.rawValue {
                        return
                    }
                    if name == AVAudioSession.routeChangeNotification {
                        let reason = (note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt)
                            .flatMap(AVAudioSession.RouteChangeReason.init(rawValue:))
                        // Category changes caused by our own activation are not
                        // device loss and must not immediately cancel startup.
                        guard reason == .oldDeviceUnavailable || reason == .newDeviceAvailable else { return }
                    }
                    self.fail("Audio interrupted. Tap Retry when ready.")
                }
            })
        }
    }

    func start(entry: RosterEntry) {
        stop()
        entryID = entry.id
        // A permission prompt is a phone setup step, never an unexpected prompt
        // on the driver's locked phone.
        guard AVAudioApplication.shared.recordPermission == .granted else {
            fail("Enable microphone access in Hi Agent on iPhone when parked.")
            return
        }
        state = .connecting
        let token = generation
        tasks.append(Task { [weak self] in
            guard let self else { return }
            do {
                let core = try await AppModel.shared.open(entry.id)
                try Task.checkCancellation()
                guard self.generation == token else { return }
                let configuration = URLSessionConfiguration.ephemeral
                configuration.httpCookieStorage = nil
                configuration.httpShouldSetCookies = false
                configuration.timeoutIntervalForRequest = 60
                configuration.timeoutIntervalForResource = 24 * 60 * 60
                let transport = URLSession(configuration: configuration, delegate: CarPlayTransportDelegate(), delegateQueue: nil)
                self.transport = transport
                let socket = transport.webSocketTask(with: try self.request(core, path: "api/in/audio/stream", webSocket: true))
                self.socket = socket
                socket.resume()
                try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
                    socket.sendPing { error in
                        if let error { continuation.resume(throwing: error) }
                        else { continuation.resume() }
                    }
                }
                try Task.checkCancellation()
                guard self.generation == token else { return }
                try self.capture(socket: socket, token: token)
                self.state = .listening
                self.tasks.append(Task { [weak self] in
                    do {
                        while !Task.isCancelled { _ = try await socket.receive() }
                    } catch { self?.failed(error, token: token) }
                })
                self.tasks.append(Task { [weak self] in
                    do { try await self?.listen(core: core, transport: transport, token: token) }
                    catch { self?.failed(error, token: token) }
                })
                self.tasks.append(Task { [weak self] in
                    do { try await self?.observeSpeech(core: core, transport: transport, token: token) }
                    catch { self?.failed(error, token: token) }
                })
                self.tasks.append(Task { [weak self] in
                    do {
                        while !Task.isCancelled {
                            try await Task.sleep(for: .seconds(2))
                            guard let self, self.generation == token else { return }
                            if Date().timeIntervalSince(self.lastFrame) > 5 {
                                self.fail("Microphone stopped. Tap Retry.")
                                return
                            }
                        }
                    } catch { }
                })
            } catch { self.failed(error, token: token) }
        })
    }

    func stop() {
        generation = UUID()
        tasks.forEach { $0.cancel() }
        tasks.removeAll()
        frames?.finish()
        frames = nil
        if let engine {
            engine.inputNode.removeTap(onBus: 0)
            engine.stop()
        }
        engine = nil
        socket?.cancel(with: .goingAway, reason: nil)
        socket = nil
        transport?.invalidateAndCancel()
        transport = nil
        player?.stop()
        player = nil
        if ownsAudio {
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        }
        state = .idle
        entryID = nil
        message = ""
    }

    private func fail(_ message: String) {
        let id = entryID
        stop()
        entryID = id
        self.message = message
        state = .failed
    }

    private func failed(_ error: Error, token: UUID) {
        guard generation == token, !Task.isCancelled else { return }
        // No server response bodies or credentials are exposed on the car screen.
        fail("Connection lost. Check the agent and tap Retry.")
    }

    private func capture(socket: URLSessionWebSocketTask, token: UUID) throws {
        let audio = AVAudioSession.sharedInstance()
        try audio.setCategory(.playAndRecord, mode: .voiceChat, options: [.allowBluetoothHFP, .defaultToSpeaker])
        try audio.setActive(true)
        let engine = AVAudioEngine()
        try engine.inputNode.setVoiceProcessingEnabled(true)
        let source = engine.inputNode.outputFormat(forBus: 0)
        let encoder = try CarPlayPCMEncoder(source: source)
        let (stream, continuation) = AsyncStream<Data>.makeStream(bufferingPolicy: .bufferingNewest(8))
        frames = continuation
        lastFrame = Date()
        engine.inputNode.installTap(onBus: 0, bufferSize: 4096, format: source) { input, _ in
            guard let data = try? encoder.encode(input), !data.isEmpty else { return }
            continuation.yield(data)
        }
        self.engine = engine
        try engine.start()
        tasks.append(Task { [weak self] in
            do {
                for await frame in stream {
                    try Task.checkCancellation()
                    guard let self, self.generation == token else { return }
                    self.lastFrame = Date()
                    try await socket.send(.data(frame))
                }
            } catch { self?.failed(error, token: token) }
        })
    }

    private func listen(core: CoreSession, transport: URLSession, token: UUID) async throws {
        while !Task.isCancelled, generation == token {
            let (bytes, response) = try await transport.bytes(for: request(core, path: "api/out/audio"))
            let http = try validate(response)
            if http.statusCode == 204 { continue }
            guard http.mimeType == "audio/mpeg" || http.mimeType == "audio/mp3" else {
                throw CoreClientError.requestFailed("Unsupported voice format")
            }
            let revision = speechRevision
            player?.stop()
            let output = try CarPlayAudioPlayer()
            player = output
            output.onFinished = { [weak self, weak output] in
                guard let self, self.generation == token, self.player === output else { return }
                self.state = .listening
            }
            var chunk = Data()
            for try await byte in bytes {
                try Task.checkCancellation()
                guard generation == token else { return }
                if revision != speechRevision { continue } // drain interrupted turn
                chunk.append(byte)
                if chunk.count >= 4096 {
                    try output.append(chunk)
                    chunk.removeAll(keepingCapacity: true)
                    state = .speaking
                }
            }
            if revision == speechRevision {
                if !chunk.isEmpty { try output.append(chunk); state = .speaking }
                output.finish()
            }
        }
    }

    private func observeSpeech(core: CoreSession, transport: URLSession, token: UUID) async throws {
        var previous = ""
        while !Task.isCancelled, generation == token {
            let (bytes, response) = try await transport.bytes(for: request(core, path: "api/out/text"))
            _ = try validate(response)
            // Read only recognition partials for barge-in; never render transcripts
            // on CarPlay. Bound lines, including the initial history snapshot.
            var line = Data()
            var oversized = false
            for try await byte in bytes {
                try Task.checkCancellation()
                guard generation == token else { return }
                if byte != 10 {
                    if line.count < 512 * 1024 { line.append(byte) } else { oversized = true }
                    continue
                }
                defer { line.removeAll(keepingCapacity: true); oversized = false }
                guard !oversized,
                      let object = try? JSONSerialization.jsonObject(with: line) as? [String: Any],
                      object.keys.contains("interim") else { continue }
                let interim = object["interim"] as? String ?? ""
                if !interim.isEmpty, interim != previous {
                    speechRevision += 1
                    player?.stop()
                    player = nil
                    state = .listening
                }
                previous = interim
            }
            try await Task.sleep(for: .seconds(1))
        }
    }

    private func request(_ core: CoreSession, path: String, webSocket: Bool = false) throws -> URLRequest {
        var url = core.baseURL.appendingPathComponent(path)
        if webSocket {
            var components = URLComponents(url: url, resolvingAgainstBaseURL: false)!
            components.scheme = url.scheme == "https" ? "wss" : "ws"
            guard let socketURL = components.url else { throw CoreClientError.invalidAddress }
            url = socketURL
        }
        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData)
        request.setValue("\(core.cookie.name)=\(core.cookie.value)", forHTTPHeaderField: "Cookie")
        request.setValue("1", forHTTPHeaderField: "X-HI-Surface")
        request.setValue(path == "api/out/text" ? "application/x-ndjson" : "audio/mpeg", forHTTPHeaderField: "Accept")
        return request
    }

    private func validate(_ response: URLResponse) throws -> HTTPURLResponse {
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            throw CoreClientError.invalidResponse
        }
        return http
    }
}

/// A session cookie is scoped to the paired core, including when a proxy returns
/// a redirect. Never forward the manually attached cookie to another origin.
private final class CarPlayTransportDelegate: NSObject, URLSessionTaskDelegate {
    func urlSession(
        _ session: URLSession, task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void
    ) {
        guard let original = task.originalRequest?.url, let redirected = request.url,
              original.scheme == redirected.scheme,
              original.host == redirected.host, original.port == redirected.port else {
            completionHandler(nil)
            return
        }
        completionHandler(request)
    }
}
