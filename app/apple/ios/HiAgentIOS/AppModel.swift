import Combine
import Foundation
import UIKit

@MainActor
final class AppModel: ObservableObject {
    @Published private(set) var entries: [RosterEntry] = []
    @Published var selectedID: String?
    @Published private(set) var isRefreshing = false
    @Published var pairingRequest: PairingRequest?
    @Published var pairingLinkError: String?
    @Published private(set) var credentialRevision = 0

    private let defaults = UserDefaults.standard
    private let keychain = KeychainStore()
    private let storageKey = "hi.agent.ios.roster.v1"

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
