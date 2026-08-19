import SwiftUI

struct ContentView: View {
    @Environment(\.scenePhase) private var scenePhase
    @EnvironmentObject private var model: AppModel

    var body: some View {
        NavigationSplitView {
            List(selection: $model.selectedID) {
                if model.entries.isEmpty {
                    ContentUnavailableView(
                        "No cores yet",
                        systemImage: "point.3.connected.trianglepath.dotted",
                        description: Text("Pair a core to open the conversation.")
                    )
                    .listRowBackground(Color.clear)
                } else {
                    Section("Cores") {
                        ForEach(model.entries) { entry in
                            CoreRow(entry: entry)
                                .tag(entry.id)
                                .contextMenu {
                                    if !entry.attached {
                                        Button {
                                            model.attach(entry.id)
                                        } label: {
                                            Label("Use this core", systemImage: "checkmark.circle")
                                        }
                                    }
                                    Button(role: .destructive) {
                                        try? model.forget(entry.id)
                                    } label: {
                                        Label("Forget", systemImage: "trash")
                                    }
                                }
                        }
                        .onDelete { offsets in
                            let ids = offsets.compactMap { offset in
                                model.entries.indices.contains(offset) ? model.entries[offset].id : nil
                            }
                            for id in ids {
                                try? model.forget(id)
                            }
                        }
                    }
                }
            }
            .navigationTitle("Hi Agent")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        model.pairingRequest = .manual
                    } label: {
                        Image(systemName: "plus")
                    }
                    .accessibilityLabel("Pair a core")
                }
            }
            .refreshable {
                await model.refresh()
            }
        } detail: {
            if let selectedID = model.selectedID, let entry = model.entry(id: selectedID) {
                CoreDetailView(entry: entry)
            } else {
                ContentUnavailableView(
                    "Choose a core",
                    systemImage: "rectangle.connected.to.line.below",
                    description: Text("Select a paired core to open its face.")
                )
            }
        }
        .sheet(item: $model.pairingRequest) { request in
            PairCoreView(request: request)
                .environmentObject(model)
        }
        .alert(
            "Could not open pairing link",
            isPresented: Binding(
                get: { model.pairingLinkError != nil },
                set: { if !$0 { model.pairingLinkError = nil } }
            )
        ) {
            Button("OK", role: .cancel) {
                model.pairingLinkError = nil
            }
        } message: {
            Text(model.pairingLinkError ?? "")
        }
        .task {
            await model.refresh()
            if model.entries.isEmpty && model.pairingRequest == nil {
                model.pairingRequest = .manual
            }
        }
        .onOpenURL { url in
            model.handleIncomingURL(url)
        }
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active else {
                return
            }
            Task {
                await model.refresh()
            }
        }
        .onChange(of: model.selectedID) { _, selectedID in
            guard let selectedID else { return }
            model.attach(selectedID)
        }
    }
}

private struct CoreRow: View {
    let entry: RosterEntry

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: entry.health.systemImage)
                .foregroundStyle(entry.health == .here ? .green : .secondary)
                .frame(width: 24)

            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 8) {
                    Text(entry.label)
                        .font(.headline)
                    if entry.attached {
                        Text("Current")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                    }
                }
                Text(entry.baseURL)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Text(entry.health.title)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(entry.label), \(entry.health.title)\(entry.attached ? ", current" : "")")
    }
}

private struct CoreDetailView: View {
    @Environment(\.scenePhase) private var scenePhase
    @EnvironmentObject private var model: AppModel
    @EnvironmentObject private var network: NetworkMonitor
    let entry: RosterEntry
    @State private var session: CoreSession?
    @State private var errorMessage: String?
    @State private var isLoading = false
    @State private var webViewState = WebViewState.loading

    var body: some View {
        ZStack {
            if let session {
                CoreWebView(session: session) { event in
                    handle(event)
                }
                .ignoresSafeArea(edges: .bottom)
            }

            if !network.isConnected {
                ConnectionStateView(
                    title: "You're offline",
                    systemImage: "wifi.slash",
                    message: "Hi Agent will reconnect when this device is back online."
                )
            } else if isLoading || (session != nil && webViewState == .loading) {
                ProgressView("Opening \(entry.label)...")
            } else if session == nil || webViewState == .failed {
                ConnectionStateView(
                    title: "Could not open this core",
                    systemImage: "wifi.exclamationmark",
                    message: errorMessage ?? "Check the core address and try again.",
                    retryTitle: "Retry",
                    onRetry: {
                        Task { await open() }
                    },
                    secondaryTitle: "Pair again",
                    onSecondary: {
                        model.pairingRequest = PairingRequest(
                            baseURL: entry.baseURL,
                            code: "",
                            label: entry.label
                        )
                    }
                )
            }
        }
        .navigationTitle(entry.label)
        .navigationBarTitleDisplayMode(.inline)
        .task(id: "\(entry.id)-\(model.credentialRevision)") {
            await open()
        }
        .onChange(of: network.isConnected) { wasConnected, isConnected in
            guard !wasConnected, isConnected else {
                return
            }
            Task {
                await open()
            }
        }
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active else {
                return
            }
            Task {
                await model.refresh(entryID: entry.id)
                if session?.needsRenewal == true || webViewState == .failed {
                    await open()
                }
            }
        }
    }

    private func open() async {
        guard network.isConnected, !isLoading else {
            return
        }
        isLoading = true
        errorMessage = nil
        webViewState = .loading
        defer { isLoading = false }
        do {
            let nextSession = try await model.open(entry.id)
            try Task.checkCancellation()
            session = nextSession
        } catch {
            guard !Task.isCancelled else {
                return
            }
            session = nil
            errorMessage = connectionMessage(for: error)
            webViewState = .failed
        }
    }

    private func handle(_ event: CoreWebViewEvent) {
        switch event {
        case .ready:
            errorMessage = nil
            webViewState = .ready
        case .sessionExpired:
            Task {
                await open()
            }
        case .failed(let error):
            errorMessage = connectionMessage(for: error)
            webViewState = .failed
        }
    }

    private func connectionMessage(for error: Error) -> String {
        let nsError = error as NSError
        if nsError.domain == NSURLErrorDomain {
            switch URLError.Code(rawValue: nsError.code) {
            case .notConnectedToInternet:
                return "This device is offline."
            case .timedOut:
                return "The core took too long to respond."
            case .cannotFindHost, .dnsLookupFailed:
                return "The core address could not be found."
            case .cannotConnectToHost, .networkConnectionLost:
                return "The connection to the core was lost."
            case .secureConnectionFailed,
                 .serverCertificateHasBadDate,
                 .serverCertificateUntrusted,
                 .serverCertificateHasUnknownRoot,
                 .serverCertificateNotYetValid:
                return "The core's secure connection could not be verified."
            default:
                break
            }
        }
        if let clientError = error as? CoreClientError,
           case .rejected(let status, _) = clientError,
           status == 401 {
            return "This device's credential was not accepted. Pair it with the core again."
        }
        return error.localizedDescription
    }

    private enum WebViewState: Equatable {
        case loading
        case ready
        case failed
    }
}

private struct ConnectionStateView: View {
    let title: String
    let systemImage: String
    let message: String
    var retryTitle: String?
    var onRetry: (() -> Void)?
    var secondaryTitle: String?
    var onSecondary: (() -> Void)?

    var body: some View {
        VStack(spacing: 18) {
            ContentUnavailableView(
                title,
                systemImage: systemImage,
                description: Text(message)
            )

            if let retryTitle, let onRetry {
                Button(retryTitle, action: onRetry)
                    .buttonStyle(.borderedProminent)
            }

            if let secondaryTitle, let onSecondary {
                Button(secondaryTitle, action: onSecondary)
                    .buttonStyle(.bordered)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.background)
    }
}

private struct PairCoreView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: AppModel
    let request: PairingRequest
    @State private var baseURL: String
    @State private var code: String
    @State private var label: String
    @State private var errorMessage: String?
    @State private var isPairing = false
    @State private var showingScanner = false

    init(request: PairingRequest) {
        self.request = request
        _baseURL = State(initialValue: request.baseURL)
        _code = State(initialValue: request.code)
        _label = State(initialValue: request.label)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Button {
                        showingScanner = true
                    } label: {
                        Label("Scan QR code", systemImage: "qrcode.viewfinder")
                    }

                    TextField("Core address", text: $baseURL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                    SecureField("Pairing code", text: $code)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Core name (optional)", text: $label)
                        .textInputAutocapitalization(.words)
                } header: {
                    Text("Pair with a core")
                } footer: {
                    Text("Use the address and one-time code shown by a device that already has access. The optional name is only for this roster.")
                }

                if let errorMessage {
                    Section {
                        Label(errorMessage, systemImage: "exclamationmark.triangle")
                            .foregroundStyle(.red)
                    }
                }

                Section {
                    Button {
                        Task { await pair() }
                    } label: {
                        HStack {
                            Text("Pair")
                            Spacer()
                            if isPairing {
                                ProgressView()
                            }
                        }
                    }
                    .disabled(isPairing || baseURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || code.isEmpty)
                }
            }
            .navigationTitle("Pair a core")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") {
                        dismiss()
                    }
                }
            }
            .fullScreenCover(isPresented: $showingScanner) {
                PairingQRScannerView { request in
                    baseURL = request.baseURL
                    code = request.code
                    if !request.label.isEmpty {
                        label = request.label
                    }
                }
            }
            .onChange(of: request.id) { _, _ in
                baseURL = request.baseURL
                code = request.code
                label = request.label
                errorMessage = nil
            }
        }
    }

    private func pair() async {
        isPairing = true
        errorMessage = nil
        defer { isPairing = false }
        do {
            try await model.pair(baseURL: baseURL, code: code, label: label)
            dismiss()
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}
