import SwiftUI

/// The root. The attached core's face *is* the app, edge to edge; the roster is
/// a place you visit, not the screen you start on.
///
/// This replaces a `NavigationSplitView` whose sidebar was the launch screen.
/// On an iPhone that put a list of one row in front of the only thing anyone
/// opens the app for, and made "no cores yet" the product's first impression.
struct ContentView: View {
    @Environment(\.scenePhase) private var scenePhase
    @EnvironmentObject private var model: AppModel
    @State private var showingRoster = false

    private var current: RosterEntry? {
        if let selectedID = model.selectedID, let entry = model.entry(id: selectedID) {
            return entry
        }
        return model.entries.first(where: { $0.attached }) ?? model.entries.first
    }

    var body: some View {
        ZStack {
            if let current {
                CoreStageView(entry: current) {
                    Haptic.tap()
                    showingRoster = true
                }
                .transition(.opacity)
                .id(current.id)
            } else {
                WelcomeView(
                    onScan: { model.pairingRequest = .scan },
                    onManual: { model.pairingRequest = .manual }
                )
                .transition(.opacity.combined(with: .scale(scale: 0.98)))
            }
        }
        .animation(.smooth(duration: 0.35), value: current?.id)
        .sheet(isPresented: $showingRoster) {
            RosterView()
                .environmentObject(model)
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
        }
        .onOpenURL { url in
            showingRoster = false
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

// MARK: - Welcome

/// First run. One mark, one sentence, one obvious thing to do — and no modal
/// thrown at the reader before they have looked at the screen.
private struct WelcomeView: View {
    let onScan: () -> Void
    let onManual: () -> Void

    @State private var appeared = false

    var body: some View {
        VStack(spacing: 22) {
            CoreMark(size: 104)
                .scaleEffect(appeared ? 1 : 0.82)
                .opacity(appeared ? 1 : 0)

            VStack(spacing: 10) {
                Text("Hi Agent")
                    .font(Theme.display(38))
                    .foregroundStyle(.primary)

                Text("Pair a core to open the conversation.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 300)
            }
            .opacity(appeared ? 1 : 0)
            .offset(y: appeared ? 0 : 10)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .hiCanvas()
        .safeAreaInset(edge: .bottom) {
            VStack(spacing: 14) {
                Button(action: onScan) {
                    Label("Scan pairing code", systemImage: "qrcode.viewfinder")
                        .font(.body.weight(.semibold))
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .buttonBorderShape(.roundedRectangle(radius: 14))

                Button("Enter details manually", action: onManual)
                    .font(.subheadline.weight(.medium))
                    .tint(Theme.ink)
            }
            .padding(.horizontal, Theme.gutter)
            .padding(.bottom, 20)
            .opacity(appeared ? 1 : 0)
            .offset(y: appeared ? 0 : 16)
        }
        .task {
            withAnimation(.smooth(duration: 0.6)) {
                appeared = true
            }
        }
    }
}

// MARK: - Stage

/// The attached core, full height, under a slim bar that names it and gets out
/// of the way. The bar is a safe-area inset rather than an overlay so it never
/// covers the face's own header.
private struct CoreStageView: View {
    @Environment(\.scenePhase) private var scenePhase
    @EnvironmentObject private var model: AppModel
    @EnvironmentObject private var network: NetworkMonitor
    let entry: RosterEntry
    let onOpenRoster: () -> Void

    @State private var session: CoreSession?
    @State private var errorMessage: String?
    @State private var isLoading = false
    @State private var webViewState = WebViewState.loading
    @State private var reloadToken = 0

    var body: some View {
        ZStack {
            if let session {
                CoreWebView(session: session, reloadToken: reloadToken) { event in
                    handle(event)
                }
                .ignoresSafeArea(edges: .bottom)
                .opacity(webViewState == .ready ? 1 : 0)
            }

            if !network.isConnected {
                StatusScreen(
                    symbol: "wifi.slash",
                    title: "You're offline",
                    message: "Hi Agent reconnects to \(entry.label) as soon as this device is back online."
                )
                .transition(.opacity)
            } else if session != nil, webViewState == .ready {
                EmptyView()
            } else if isLoading || webViewState == .loading {
                LoadingVeil(label: entry.label)
                    .transition(.opacity)
            } else {
                StatusScreen(
                    symbol: "antenna.radiowaves.left.and.right.slash",
                    title: "Can't reach this core",
                    message: errorMessage ?? "Check the core address and try again.",
                    primary: .init(title: "Try again", action: { Task { await open() } }),
                    secondary: .init(title: "Pair again", action: {
                        model.pairingRequest = PairingRequest(
                            baseURL: entry.baseURL,
                            code: "",
                            label: entry.label
                        )
                    })
                )
                .transition(.opacity)
            }
        }
        .animation(.easeInOut(duration: 0.25), value: webViewState)
        .animation(.easeInOut(duration: 0.25), value: network.isConnected)
        .hiCanvas()
        .safeAreaInset(edge: .top, spacing: 0) {
            StageBar(
                entry: entry,
                onOpenRoster: onOpenRoster,
                onReload: {
                    Haptic.tap()
                    if webViewState == .ready {
                        reloadToken += 1
                    } else {
                        Task { await open() }
                    }
                }
            )
        }
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
        if let keychainError = error as? KeychainError, case .read = keychainError {
            // The OSStatus is diagnostic noise to the reader; what they can act
            // on is that the stored credential is gone.
            return "This device is no longer paired with \(entry.label). Pair it again."
        }
        return error.localizedDescription
    }

    private enum WebViewState: Equatable {
        case loading
        case ready
        case failed
    }
}

/// The only chrome over the face: which core you are talking to, whether it is
/// answering, and the two things you might want — switch, or reload.
private struct StageBar: View {
    let entry: RosterEntry
    let onOpenRoster: () -> Void
    let onReload: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Button(action: onOpenRoster) {
                HStack(spacing: 6) {
                    StatusDot(state: entry.health, diameter: 8)
                    Text(entry.label)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    Image(systemName: "chevron.down")
                        .font(.system(size: 11, weight: .bold))
                        .foregroundStyle(.secondary)
                }
                .padding(.leading, 6)
                .padding(.trailing, 12)
                .padding(.vertical, 7)
                .contentShape(Capsule())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("\(entry.label), \(entry.health.title). Switch core")

            Spacer(minLength: 0)

            Button(action: onReload) {
                Image(systemName: "arrow.clockwise")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 32, height: 32)
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Reload the face")
        }
        .padding(.horizontal, 10)
        .padding(.bottom, 6)
        .background(.bar)
        .overlay(alignment: .bottom) {
            Divider().opacity(0.5)
        }
    }
}

/// Shown while a core is being opened. It waits before appearing, because a
/// spinner that flashes for 120ms reads as a glitch rather than as progress.
private struct LoadingVeil: View {
    let label: String
    @State private var visible = false

    var body: some View {
        VStack(spacing: 16) {
            CoreMark(size: 72)
            Text("Opening \(label)")
                .font(.subheadline)
                .foregroundStyle(.secondary)
            ProgressView()
                .controlSize(.small)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .hiCanvas()
        .opacity(visible ? 1 : 0)
        .task {
            try? await Task.sleep(for: .milliseconds(350))
            withAnimation(.easeOut(duration: 0.3)) {
                visible = true
            }
        }
    }
}

/// A full-screen native state — offline, unreachable — as a centred card on the
/// canvas rather than a bare `ContentUnavailableView` with buttons stacked
/// under it.
struct StatusScreen: View {
    struct Action {
        let title: String
        let action: () -> Void
    }

    let symbol: String
    let title: String
    let message: String
    var primary: Action?
    var secondary: Action?

    var body: some View {
        VStack {
            Spacer(minLength: 0)

            Card(padding: 26) {
                VStack(spacing: 14) {
                    Image(systemName: symbol)
                        .font(.system(size: 30, weight: .regular))
                        .foregroundStyle(Theme.ink)
                        .padding(.bottom, 2)

                    Text(title)
                        .font(Theme.display(20, .semibold))
                        .multilineTextAlignment(.center)

                    Text(message)
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)

                    if primary != nil || secondary != nil {
                        VStack(spacing: 10) {
                            if let primary {
                                Button(action: primary.action) {
                                    Text(primary.title)
                                        .font(.body.weight(.semibold))
                                        .frame(maxWidth: .infinity)
                                }
                                .buttonStyle(.borderedProminent)
                                .controlSize(.large)
                                .buttonBorderShape(.roundedRectangle(radius: 12))
                            }
                            if let secondary {
                                Button(secondary.title, action: secondary.action)
                                    .font(.subheadline.weight(.medium))
                                    .tint(Theme.ink)
                            }
                        }
                        .padding(.top, 6)
                    }
                }
                .frame(maxWidth: .infinity)
            }
            .padding(.horizontal, Theme.gutter)

            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .hiCanvas()
    }
}
