import Foundation

enum AddAgentRequestError: LocalizedError {
    case invalidLink
    case missingAddress
    case missingCode

    var errorDescription: String? {
        switch self {
        case .invalidLink:
            return "This is not a Hi Agent link."
        case .missingAddress:
            return "That link does not contain an agent's address."
        case .missingCode:
            return "That link does not contain a one-time code."
        }
    }
}

/// What opened the add sheet, and what it should arrive holding.
///
/// The `hiagent://pair` URL this parses is **unchanged** — that is the wire, and
/// four other clients mint and read it. Only the words on this side moved.
struct AddAgentRequest: Identifiable, Equatable {
    let id = UUID()
    let baseURL: String
    let code: String
    let label: String
    /// Arrive with the camera already up rather than making the reader find the
    /// button. Set when the person picked "scan" from somewhere else.
    let opensScanner: Bool

    /// The ordinary way in: a name, and a person to approve it.
    static var ask: AddAgentRequest {
        AddAgentRequest(baseURL: "", code: "", label: "")
    }

    static var scan: AddAgentRequest {
        AddAgentRequest(baseURL: "", code: "", label: "", opensScanner: true)
    }

    init(baseURL: String, code: String, label: String, opensScanner: Bool = false) {
        self.baseURL = baseURL
        self.code = code
        self.label = label
        self.opensScanner = opensScanner
    }

    /// Whether this URL is addressed to us at all.
    ///
    /// `onOpenURL` is not a pairing-only door. iOS hands it Universal Links too —
    /// which the comment in `ContentView` asserted it did not, and the code was
    /// written to that assertion. On 2026-09-18 the share extension's own hand-off,
    /// `https://hi-agent.xyz/ios-share/<id>`, arrived here after staging a photo
    /// perfectly, and this parser answered a link that had worked with **"This is
    /// not a Hi Agent link."** So the scheme is asked about *before* parsing: a URL
    /// on somebody else's scheme is another door's business, and saying nothing is
    /// the whole of the correct response to it.
    static func handles(_ url: URL) -> Bool {
        url.scheme?.lowercased() == "hiagent"
    }

    init(url: URL) throws {
        guard Self.handles(url),
              let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
              components.host?.lowercased() == "pair"
        else {
            throw AddAgentRequestError.invalidLink
        }

        let items = components.queryItems ?? []
        guard let rawBaseURL = Self.singleValue(named: "url", in: items),
              !rawBaseURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            throw AddAgentRequestError.missingAddress
        }
        guard let rawCode = Self.singleValue(named: "code", in: items),
              !rawCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            throw AddAgentRequestError.missingCode
        }

        baseURL = try CoreClient.normalizeBaseURL(rawBaseURL).absoluteString
        code = rawCode.trimmingCharacters(in: .whitespacesAndNewlines)
        label = Self.singleValue(named: "label", in: items)?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        opensScanner = false
    }

    private static func singleValue(named name: String, in items: [URLQueryItem]) -> String? {
        let matches = items.filter { $0.name == name }
        guard matches.count == 1 else {
            return nil
        }
        return matches[0].value
    }
}
