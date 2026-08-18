import Foundation

enum HealthState: String, Codable {
    case checking
    case here
    case asleep
    case unreachable
    case unknown

    var title: String {
        switch self {
        case .checking:
            return "Checking"
        case .here:
            return "Available"
        case .asleep:
            return "Asleep"
        case .unreachable:
            return "Unreachable"
        case .unknown:
            return "Unknown"
        }
    }

    var systemImage: String {
        switch self {
        case .checking:
            return "ellipsis.circle"
        case .here:
            return "checkmark.circle.fill"
        case .asleep:
            return "moon.zzz.fill"
        case .unreachable:
            return "wifi.exclamationmark"
        case .unknown:
            return "questionmark.circle"
        }
    }
}

struct RosterEntry: Codable, Identifiable, Equatable {
    let id: String
    var label: String
    let baseURL: String
    let addedAt: String
    var attached: Bool
    var health: HealthState = .unknown

    enum CodingKeys: String, CodingKey {
        case id
        case label
        case baseURL
        case addedAt
        case attached
    }

    init(
        id: String,
        label: String,
        baseURL: String,
        addedAt: String = ISO8601DateFormatter().string(from: Date()),
        attached: Bool = false,
        health: HealthState = .unknown
    ) {
        self.id = id
        self.label = label
        self.baseURL = baseURL
        self.addedAt = addedAt
        self.attached = attached
        self.health = health
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id)
        label = try values.decode(String.self, forKey: .label)
        baseURL = try values.decode(String.self, forKey: .baseURL)
        addedAt = try values.decode(String.self, forKey: .addedAt)
        attached = try values.decode(Bool.self, forKey: .attached)
        health = .unknown
    }
}

struct SessionExchange: Decodable {
    let id: String
    let credential: String?
}

struct CoreSession {
    let entryID: String
    let baseURL: URL
    let cookie: HTTPCookie
}
