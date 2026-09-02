import Combine
import Foundation
import UIKit
import UniformTypeIdentifiers

@MainActor
final class AppModel: ObservableObject {
    /// The one roster in the process. A `@StateObject` alone was enough while the
    /// only way into the app was its own window; [`ShowScreenIntent`] runs in this
    /// process too but outside SwiftUI, with no environment to be handed, and it
    /// must reach the *same* roster the screen is showing.
    static let shared = AppModel()

    @Published private(set) var entries: [RosterEntry] = []
    @Published var selectedID: String?
    @Published private(set) var isRefreshing = false
    @Published var pairingRequest: PairingRequest?
    @Published var pairingLinkError: String?
    @Published private(set) var credentialRevision = 0
    /// Where the last screen the person showed got to, or `nil` if they have not
    /// shown one this launch.
    @Published private(set) var showScreenState: ShowScreenState?

    private let defaults = UserDefaults.standard
    private let keychain = KeychainStore()
    private let storageKey = "hi.agent.ios.roster.v1"
    /// The bytes behind a `.failed` state, kept so "Try again" is a retry and not a
    /// request that the person go back and make the gesture a second time. Dropped
    /// as soon as it lands.
    private var pendingScreen: PendingScreen?

    init() {
        load()
    }

    func entry(id: String) -> RosterEntry? {
        entries.first(where: { $0.id == id })
    }

    func handleIncomingURL(_ url: URL) {
        do {
            pairingRequest = try PairingRequest(url: url)
            pairingLinkError = nil
        } catch {
            pairingLinkError = error.localizedDescription
        }
    }

    func pair(baseURL rawBaseURL: String, code rawCode: String, label rawLabel: String) async throws {
        let baseURL = try CoreClient.normalizeBaseURL(rawBaseURL)
        let code = rawCode.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !code.isEmpty else {
            throw CoreClientError.requestFailed("Enter the pairing code from the core.")
        }
        let requestedLabel = rawLabel.trimmingCharacters(in: .whitespacesAndNewlines)
        let coreLabel = requestedLabel.isEmpty ? defaultCoreLabel(for: baseURL) : requestedLabel

        let result = try await CoreClient.exchange(
            at: baseURL,
            presented: code,
            label: UIDevice.current.name
        )
        let credential = result.exchange.credential ?? code
        try keychain.save(credential, account: credentialAccount(id: result.exchange.id))
        credentialRevision += 1

        let wasAttached = entries.first(where: { $0.id == result.exchange.id })?.attached
            ?? entries.isEmpty
        let entry = RosterEntry(
            id: result.exchange.id,
            label: coreLabel,
            baseURL: baseURL.absoluteString,
            attached: wasAttached
        )
        entries.removeAll(where: { $0.id == entry.id })
        entries.append(entry)
        if entry.attached {
            selectedID = entry.id
        }
        persist()
        await refresh(entryID: entry.id)
    }

    func attach(_ id: String) {
        guard entries.contains(where: { $0.id == id }) else {
            return
        }
        entries = entries.map { entry in
            var updated = entry
            updated.attached = entry.id == id
            return updated
        }
        selectedID = id
        persist()
    }

    func forget(_ id: String) throws {
        let removedWasAttached = entries.first(where: { $0.id == id })?.attached == true
        try keychain.delete(account: credentialAccount(id: id))
        entries.removeAll(where: { $0.id == id })
        if removedWasAttached, !entries.isEmpty {
            entries[0].attached = true
        }
        if selectedID == id || selectedID == nil {
            selectedID = entries.first(where: { $0.attached })?.id ?? entries.first?.id
        }
        persist()
    }

    func refresh() async {
        isRefreshing = true
        defer { isRefreshing = false }
        for entry in entries {
            await refresh(entryID: entry.id)
        }
    }

    func open(_ id: String) async throws -> CoreSession {
        guard let entry = entry(id: id),
              let baseURL = URL(string: entry.baseURL)
        else {
            throw CoreClientError.invalidAddress
        }
        let credential = try keychain.read(account: credentialAccount(id: id))
        let result = try await CoreClient.exchange(at: baseURL, presented: credential, label: entry.label)
        if let index = entries.firstIndex(where: { $0.id == id }) {
            entries[index].health = .here
        }
        return CoreSession(entryID: id, baseURL: baseURL, cookie: result.cookie)
    }

    func refresh(entryID: String) async {
        guard let index = entries.firstIndex(where: { $0.id == entryID }),
              let baseURL = URL(string: entries[index].baseURL)
        else {
            return
        }
        entries[index].health = .checking
        let state = await CoreClient.health(at: baseURL)
        guard let index = entries.firstIndex(where: { $0.id == entryID }) else {
            return
        }
        entries[index].health = state
    }

    // MARK: - Showing the screen

    /// Hand a screenshot to the attached core (see [`ShowScreen`]). Called from the
    /// intent, so it reports rather than throws: the app is already coming to the
    /// front, and the banner is where a person can act on what happened.
    func showScreen(data: Data, type: UTType?, note: String?) async {
        let screen = PendingScreen(
            data: data,
            filename: ShowScreen.filename(for: type, at: Date()),
            mime: ShowScreen.mime(for: type),
            note: note ?? ShowScreen.note
        )
        await send(screen)
    }

    /// Try the kept screen again. No-op once it has landed.
    func retryShowScreen() async {
        guard let screen = pendingScreen else {
            return
        }
        await send(screen)
    }

    func dismissShowScreen() {
        pendingScreen = nil
        showScreenState = nil
    }

    private func send(_ screen: PendingScreen) async {
        guard let entry = attachedEntry else {
            pendingScreen = screen
            showScreenState = .failed(
                reason: "This device isn't paired with a core yet, so there's nobody to show it to."
            )
            return
        }
        guard let baseURL = URL(string: entry.baseURL) else {
            pendingScreen = screen
            showScreenState = .failed(reason: CoreClientError.invalidAddress.localizedDescription)
            return
        }

        pendingScreen = screen
        showScreenState = .sending
        do {
            let credential = try keychain.read(account: credentialAccount(id: entry.id))
            try await CoreClient.hand(
                at: baseURL,
                credential: credential,
                data: screen.data,
                filename: screen.filename,
                mime: screen.mime,
                note: screen.note
            )
            pendingScreen = nil
            showScreenState = .sent(coreLabel: entry.label)
        } catch let error as KeychainError {
            if case .read = error {
                showScreenState = .failed(
                    reason: "This device is no longer paired with \(entry.label). Pair it again."
                )
            } else {
                showScreenState = .failed(reason: error.localizedDescription)
            }
        } catch {
            showScreenState = .failed(reason: error.localizedDescription)
        }
    }

    /// The core everything this device does goes to — the same one the stage shows.
    var attachedEntry: RosterEntry? {
        if let selectedID, let entry = entry(id: selectedID) {
            return entry
        }
        return entries.first(where: { $0.attached }) ?? entries.first
    }

    private func load() {
        guard let data = defaults.data(forKey: storageKey) else {
            return
        }
        do {
            entries = try JSONDecoder().decode([RosterEntry].self, from: data)
            selectedID = entries.first(where: { $0.attached })?.id
        } catch {
            defaults.removeObject(forKey: storageKey)
        }
    }

    private func persist() {
        do {
            defaults.set(try JSONEncoder().encode(entries), forKey: storageKey)
        } catch {
            // The roster contains only small local metadata. A failed write should
            // not discard the in-memory selection or the credential in Keychain.
        }
    }

    private func credentialAccount(id: String) -> String {
        "credential.\(id)"
    }

    private func defaultCoreLabel(for baseURL: URL) -> String {
        let host = baseURL.host ?? "Core"
        let path = baseURL.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        return path.isEmpty ? host : "\(host)/\(path)"
    }
}
