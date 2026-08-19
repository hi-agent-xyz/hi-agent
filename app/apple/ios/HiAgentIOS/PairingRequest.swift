import Foundation

enum PairingRequestError: LocalizedError {
    case invalidLink
    case missingAddress
    case missingCode

    var errorDescription: String? {
        switch self {
        case .invalidLink:
            return "This is not a Hi Agent pairing link."
        case .missingAddress:
            return "The pairing link does not contain a core address."
        case .missingCode:
            return "The pairing link does not contain a pairing code."
        }
    }
}

struct PairingRequest: Identifiable, Equatable {
    let id = UUID()
    let baseURL: String
    let code: String
    let label: String

    static var manual: PairingRequest {
        PairingRequest(baseURL: "", code: "", label: "")
    }

    init(baseURL: String, code: String, label: String) {
        self.baseURL = baseURL
        self.code = code
        self.label = label
    }

    init(url: URL) throws {
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
              components.scheme?.lowercased() == "hiagent",
              components.host?.lowercased() == "pair"
        else {
            throw PairingRequestError.invalidLink
        }

        let items = components.queryItems ?? []
        guard let rawBaseURL = Self.singleValue(named: "url", in: items),
              !rawBaseURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            throw PairingRequestError.missingAddress
        }
        guard let rawCode = Self.singleValue(named: "code", in: items),
              !rawCode.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            throw PairingRequestError.missingCode
        }

        baseURL = try CoreClient.normalizeBaseURL(rawBaseURL).absoluteString
        code = rawCode.trimmingCharacters(in: .whitespacesAndNewlines)
        label = Self.singleValue(named: "label", in: items)?
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    }

    private static func singleValue(named name: String, in items: [URLQueryItem]) -> String? {
        let matches = items.filter { $0.name == name }
        guard matches.count == 1 else {
            return nil
        }
        return matches[0].value
    }
}
