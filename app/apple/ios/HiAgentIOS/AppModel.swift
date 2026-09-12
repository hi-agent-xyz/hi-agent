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
    @Published var addRequest: AddAgentRequest?
    @Published var addLinkError: String?
    @Published private(set) var credentialRevision = 0
    /// Where the last thing the person handed over got to, or `nil` if they have not
    /// handed anything over this launch. The bytes behind a failure are not here —
    /// they are in the queue on disk ([`HandedDrop`]), which is what makes "Try
    /// again" survive the app being killed.
    @Published private(set) var handoffState: HandoffState?

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
            addRequest = try AddAgentRequest(url: url)
            addLinkError = nil
        } catch {
            addLinkError = error.localizedDescription
        }
    }

    /// Ask an agent, by name, to let this device in.
    ///
    /// Nothing is stored by this call: what comes back is a code to show and a
    /// secret to wait on. The roster only grows once [`join`] lands.
    func askToJoin(name: String) async throws -> JoinInvitation {
        let baseURL = try CoreClient.address(forName: name)
        let ticket = try await CoreClient.askToJoin(at: baseURL, label: UIDevice.current.name)
        return JoinInvitation(
            baseURL: baseURL,
            label: agentLabel(for: name, at: baseURL),
            code: ticket.code,
            secret: ticket.secret
        )
    }

    /// Wait out one invitation, and take the credential the moment it is approved.
    ///
    /// Polls until answered or cancelled — the caller's `Task` is what ends it, so
    /// closing the sheet stops the wait. Past the request's own ten-minute life the
    /// agent answers `expired` and this throws, which is the same clock rather than
    /// a second one kept here.
    func join(_ invitation: JoinInvitation) async throws {
        while true {
            try Task.checkCancellation()
            switch try await CoreClient.pollJoin(at: invitation.baseURL, secret: invitation.secret) {
            case .approved(let credential):
                // From here it is the ordinary add: the credential goes to the
                // keychain, the agent joins the roster, and the stage opens it.
                try await add(
                    baseURL: invitation.baseURL.absoluteString,
                    code: credential,
                    label: invitation.label
                )
                return
            case .denied:
                throw CoreClientError.requestFailed("That was turned down.")
            case .expired:
                throw CoreClientError.requestFailed("Nobody answered in time. Ask again.")
            case .waiting:
                try await Task.sleep(for: .seconds(2))
            }
        }
    }

    func add(baseURL rawBaseURL: String, code rawCode: String, label rawLabel: String) async throws {
        let baseURL = try CoreClient.normalizeBaseURL(rawBaseURL)
        let code = rawCode.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !code.isEmpty else {
            throw CoreClientError.requestFailed("Enter the one-time code the agent is showing.")
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

    // MARK: - Handing things over

    /// Hand a screenshot to the attached core (see [`ShowScreen`]).
    ///
    /// It goes onto the same queue a share does rather than straight out over the
    /// wire. The app is on screen either way, so the queue buys one thing here: a
    /// screen that failed to send is still on disk tomorrow. It used to live in a
    /// property and die with the process.
    func showScreen(data: Data, type: UTType?, note: String?) async {
        do {
            let drop = try HandedDrop.enqueue(
                note: note ?? ShowScreen.note,
                text: nil,
                files: [
                    HandedDrop.PendingFile(
                        filename: ShowScreen.filename(for: type, at: Date()),
                        mime: ShowScreen.mime(for: type),
                        source: .data(data)
                    )
                ]
            )
            // `nil` means no group container, which is a missing entitlement rather
            // than a runtime condition. Said out loud because the alternative is the
            // worst shape a failure can have here: the gesture appears to work, and
            // nothing ever arrives.
            guard drop != nil else {
                handoffState = .failed(reason: HandedDropError.noContainer.localizedDescription)
                return
            }
        } catch {
            handoffState = .failed(reason: error.localizedDescription)
            return
        }
        await deliverQueued()
    }

    /// Send everything waiting, oldest first.
    ///
    /// Called when the app comes forward, and on the `hiagent://shared` the share
    /// extension opens — **not on a timer and not from a background task.** The queue
    /// is drained where its result can be seen, because a failure that nobody is
    /// looking at is a file that quietly stops existing.
    ///
    /// Reports rather than throws: the caller is a scene phase or a URL, neither of
    /// which has anywhere to put an error. The banner does.
    func deliverQueued() async {
        let drops = HandedDrop.queued()
        guard !drops.isEmpty else {
            return
        }
        guard let entry = attachedEntry else {
            handoffState = .failed(
                reason: "This device hasn't been added to an agent yet, so there's nobody to hand it to."
            )
            return
        }
        guard let baseURL = URL(string: entry.baseURL) else {
            handoffState = .failed(reason: CoreClientError.invalidAddress.localizedDescription)
            return
        }

        let credential: String
        do {
            credential = try keychain.read(account: credentialAccount(id: entry.id))
        } catch let error as KeychainError {
            if case .read = error {
                handoffState = .failed(
                    reason: "This device no longer has access to \(entry.label). Add it again."
                )
            } else {
                handoffState = .failed(reason: error.localizedDescription)
            }
            return
        } catch {
            handoffState = .failed(reason: error.localizedDescription)
            return
        }

        handoffState = .sending(count: drops.reduce(0) { $0 + $1.itemCount })
        var sent = 0
        var failure: String?
        for drop in drops {
            do {
                // Words first. When a drop has both — a link shared with a picture of
                // it — the sentence lands ahead of the artifact, which is the order
                // the person did it in.
                if let text = drop.manifest.text, !text.isEmpty {
                    try await CoreClient.say(at: baseURL, credential: credential, text: text)
                    sent += 1
                }
                for part in drop.manifest.parts {
                    try await CoreClient.hand(
                        at: baseURL,
                        credential: credential,
                        body: drop.url(for: part),
                        boundary: drop.manifest.boundary
                    )
                    sent += 1
                }
                // Only now: a drop that died halfway is retried whole rather than
                // half-delivered and forgotten. The cost of that choice is a possible
                // duplicate; the cost of the other is a file the person believes they
                // sent.
                drop.discard()
            } catch {
                // **Kept, and the others are still tried.** Nothing here can tell a
                // refusal apart from a tunnel, and discarding on the wrong guess
                // throws away something a person chose to send. Carrying on past it
                // is the other half: a drop that can never land must not become a
                // wall that everything shared afterwards queues up behind.
                failure = failure ?? error.localizedDescription
            }
        }
        if let failure {
            handoffState = .failed(reason: failure)
            return
        }
        handoffState = .sent(coreLabel: entry.label, count: sent)
    }

    /// Try the queue again. The drops are still on disk, so this is a retry and not a
    /// request that the person go back and share it a second time.
    func retryHandoff() async {
        await deliverQueued()
    }

    func dismissHandoff() {
        handoffState = nil
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
        let host = baseURL.host ?? "Agent"
        let path = baseURL.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        return path.isEmpty ? host : "\(host)/\(path)"
    }

    /// What to call an agent added by name. The name itself when that is what was
    /// typed — "iloahz" reads better in the roster than "iloahz.hi-agent.xyz" — and
    /// the host when somebody pasted a whole address.
    private func agentLabel(for name: String, at baseURL: URL) -> String {
        let typed = name.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let bare = typed.hasPrefix("@") ? String(typed.dropFirst()) : typed
        if !bare.isEmpty, !bare.contains("."), !bare.contains("/") {
            return bare
        }
        return defaultCoreLabel(for: baseURL)
    }
}

/// One outstanding "let me in", held only while the sheet is open.
struct JoinInvitation: Equatable {
    let baseURL: URL
    let label: String
    /// Shown here and on the agent's Reach view, for the two to be compared. It
    /// authorizes nothing.
    let code: String
    let secret: String
}
