import Foundation

enum CoreClientError: LocalizedError {
    case invalidAddress
    case invalidResponse
    case missingSessionCookie
    case requestFailed(String)
    case rejected(status: Int, detail: String)

    var errorDescription: String? {
        switch self {
        case .invalidAddress:
            return "Enter a core address beginning with http:// or https://."
        case .invalidResponse:
            return "The core returned an invalid response."
        case .missingSessionCookie:
            return "The core did not return a session cookie."
        case .requestFailed(let detail):
            return detail
        case .rejected(let status, let detail):
            if detail.isEmpty {
                return "The core rejected the request (HTTP \(status))."
            }
            return "The core rejected the request (HTTP \(status)): \(detail)"
        }
    }
}

enum CoreClient {
    private static let sessionCookieName = "hi_surface"
    private static let urlSession: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpShouldSetCookies = false
        configuration.httpCookieStorage = nil
        return URLSession(configuration: configuration)
    }()

    static func normalizeBaseURL(_ raw: String) throws -> URL {
        let value = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard var components = URLComponents(string: value),
              let scheme = components.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              components.host != nil,
              components.user == nil,
              components.password == nil
        else {
            throw CoreClientError.invalidAddress
        }

        components.query = nil
        components.fragment = nil
        components.path = normalizedPath(components.path)
        guard let url = components.url else {
            throw CoreClientError.invalidAddress
        }
        return url
    }

    static func exchange(
        at baseURL: URL,
        presented: String,
        label: String
    ) async throws -> (exchange: SessionExchange, cookie: HTTPCookie) {
        var request = URLRequest(url: endpoint(baseURL, path: "api/session"))
        request.httpMethod = "POST"
        request.setValue("Bearer \(presented)", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONSerialization.data(withJSONObject: [
            "label": label.trimmingCharacters(in: .whitespacesAndNewlines)
        ])

        let (data, response) = try await urlSession.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw CoreClientError.invalidResponse
        }
        guard (200..<300).contains(http.statusCode) else {
            throw CoreClientError.rejected(
                status: http.statusCode,
                detail: String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            )
        }

        let exchange: SessionExchange
        do {
            exchange = try JSONDecoder().decode(SessionExchange.self, from: data)
        } catch {
            throw CoreClientError.requestFailed("The core returned an unexpected session response.")
        }

        guard let rawCookie = http.value(forHTTPHeaderField: "Set-Cookie"),
              let cookie = HTTPCookie.cookies(
                withResponseHeaderFields: ["Set-Cookie": rawCookie],
                for: baseURL
              ).first(where: { $0.name == sessionCookieName })
        else {
            throw CoreClientError.missingSessionCookie
        }
        return (exchange, cookie)
    }

    /// Hand a file to a core through its `file` channel — the same door a drag-drop
    /// onto the face goes through (`POST /api/in/file`). `note` is the line the
    /// person effectively said as they handed it over; the core makes it their own
    /// message just ahead of the artifact.
    ///
    /// The long-lived credential is presented directly as a Bearer token instead of
    /// being exchanged for a session first. A session exists so a `WKWebView` can
    /// carry something a cookie jar understands; this is one request from native
    /// code, and exchanging first would cost a round trip and a CSRF header to say
    /// exactly what the Bearer already says.
    static func hand(
        at baseURL: URL,
        credential: String,
        data: Data,
        filename: String,
        mime: String,
        note: String?
    ) async throws {
        let boundary = "hi-agent.\(UUID().uuidString)"
        var request = URLRequest(url: endpoint(baseURL, path: "api/in/file"))
        request.httpMethod = "POST"
        request.setValue("Bearer \(credential)", forHTTPHeaderField: "Authorization")
        request.setValue(
            "multipart/form-data; boundary=\(boundary)",
            forHTTPHeaderField: "Content-Type"
        )
        // A screenshot over a phone connection is not a 4-second request; the health
        // check's timeout would fail a send that was going to land.
        request.timeoutInterval = 60
        request.httpBody = multipart(
            boundary: boundary,
            note: note,
            data: data,
            filename: filename,
            mime: mime
        )

        let (body, response) = try await urlSession.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw CoreClientError.invalidResponse
        }
        guard (200..<300).contains(http.statusCode) else {
            throw CoreClientError.rejected(
                status: http.statusCode,
                detail: String(data: body, encoding: .utf8)?
                    .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            )
        }
    }

    /// One multipart body: the note first — the core spends it on the file that
    /// follows — then the bytes.
    ///
    /// Hand-rolled because `URLSession` has no multipart encoder and this body has
    /// one shape. `filename` and `mime` are never user text: the caller builds the
    /// name and reads the type from the file's UTI, so neither can carry a quote or
    /// a newline into a header.
    private static func multipart(
        boundary: String,
        note: String?,
        data: Data,
        filename: String,
        mime: String
    ) -> Data {
        var body = Data()
        func append(_ text: String) {
            body.append(Data(text.utf8))
        }
        if let note, !note.isEmpty {
            append("--\(boundary)\r\n")
            append("Content-Disposition: form-data; name=\"note\"\r\n\r\n")
            append(note)
            append("\r\n")
        }
        append("--\(boundary)\r\n")
        append("Content-Disposition: form-data; name=\"file\"; filename=\"\(filename)\"\r\n")
        append("Content-Type: \(mime)\r\n\r\n")
        body.append(data)
        append("\r\n--\(boundary)--\r\n")
        return body
    }

    static func health(at baseURL: URL) async -> HealthState {
        var request = URLRequest(url: endpoint(baseURL, path: "healthz"))
        request.httpMethod = "GET"
        request.timeoutInterval = 4
        do {
            let (_, response) = try await urlSession.data(for: request)
            guard let http = response as? HTTPURLResponse else {
                return .unknown
            }
            if http.statusCode == 200 {
                return .here
            }
            if http.statusCode == 503 {
                return .asleep
            }
            return .unknown
        } catch {
            return .unreachable
        }
    }

    private static func endpoint(_ baseURL: URL, path: String) -> URL {
        var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false)!
        let basePath = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        components.path = "/" + ([basePath, path].filter { !$0.isEmpty }.joined(separator: "/"))
        components.query = nil
        components.fragment = nil
        return components.url!
    }

    private static func normalizedPath(_ path: String) -> String {
        let trimmed = path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        return trimmed.isEmpty ? "" : "/" + trimmed
    }
}
