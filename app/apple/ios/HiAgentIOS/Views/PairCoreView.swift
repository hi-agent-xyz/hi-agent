import SwiftUI

/// Pairing, scan-first. The QR code is the path a core actually offers, so it
/// leads; typing an address and a code is the fallback underneath it, not the
/// form the screen opens as.
struct PairCoreView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: AppModel
    let request: PairingRequest

    @State private var baseURL: String
    @State private var code: String
    @State private var label: String
    @State private var errorMessage: String?
    @State private var isPairing = false
    @State private var showingScanner = false
    @FocusState private var focus: Field?

    init(request: PairingRequest) {
        self.request = request
        _baseURL = State(initialValue: request.baseURL)
        _code = State(initialValue: request.code)
        _label = State(initialValue: request.label)
    }

    private var canPair: Bool {
        !baseURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(spacing: 22) {
                    hero
                    scanButton
                    separator
                    fields
                    if let errorMessage {
                        errorBanner(errorMessage)
                    }
                }
                .padding(.horizontal, Theme.gutter)
                .padding(.top, 8)
                .padding(.bottom, 24)
                .animation(.smooth(duration: 0.25), value: errorMessage)
            }
            .scrollDismissesKeyboard(.interactively)
            .hiCanvas()
            .navigationTitle("Pair a core")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
            .safeAreaInset(edge: .bottom) {
                pairBar
            }
            .fullScreenCover(isPresented: $showingScanner) {
                PairingQRScannerView { scanned in
                    baseURL = scanned.baseURL
                    code = scanned.code
                    if !scanned.label.isEmpty {
                        label = scanned.label
                    }
                    errorMessage = nil
                    Haptic.tap()
                }
            }
            .onChange(of: request.id) { _, _ in
                baseURL = request.baseURL
                code = request.code
                label = request.label
                errorMessage = nil
            }
            .task {
                guard request.opensScanner else { return }
                showingScanner = true
            }
        }
    }

    // MARK: Pieces

    private var hero: some View {
        VStack(spacing: 14) {
            Image(systemName: "qrcode.viewfinder")
                .font(.system(size: 40, weight: .regular))
                .foregroundStyle(Theme.ink)
                .frame(width: 84, height: 84)
                .background(
                    RoundedRectangle(cornerRadius: 24, style: .continuous)
                        .fill(Theme.ink.opacity(0.10))
                )

            Text("Point this device at the pairing code your core is showing.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 320)
        }
        .padding(.top, 10)
    }

    private var scanButton: some View {
        Button {
            focus = nil
            showingScanner = true
        } label: {
            Label("Scan QR code", systemImage: "camera.viewfinder")
                .font(.body.weight(.semibold))
                .frame(maxWidth: .infinity)
        }
        .buttonStyle(.borderedProminent)
        .controlSize(.large)
        .buttonBorderShape(.roundedRectangle(radius: 14))
    }

    private var separator: some View {
        HStack(spacing: 12) {
            Rectangle().fill(Theme.hairline).frame(height: 1)
            Text("or enter it by hand")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .fixedSize()
            Rectangle().fill(Theme.hairline).frame(height: 1)
        }
    }

    private var fields: some View {
        Card(padding: 0) {
            VStack(spacing: 0) {
                FieldRow(symbol: "link", title: "Core address") {
                    TextField("hi.example.com", text: $baseURL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                        .textContentType(.URL)
                        .submitLabel(.next)
                        .focused($focus, equals: .address)
                        .onSubmit { focus = .code }
                }

                rowDivider

                FieldRow(symbol: "key", title: "Pairing code") {
                    TextField("one-time code", text: $code)
                        .font(.body.monospaced())
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .textContentType(.oneTimeCode)
                        .submitLabel(.next)
                        .focused($focus, equals: .code)
                        .onSubmit { focus = .label }
                }

                rowDivider

                FieldRow(symbol: "tag", title: "Name", optional: true) {
                    TextField("only shown on this device", text: $label)
                        .textInputAutocapitalization(.words)
                        .submitLabel(.done)
                        .focused($focus, equals: .label)
                        .onSubmit { focus = nil }
                }
            }
        }
    }

    private var rowDivider: some View {
        Rectangle()
            .fill(Theme.hairline)
            .frame(height: 1)
            .padding(.leading, 52)
    }

    private func errorBanner(_ message: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 14, weight: .semibold))
            Text(message)
                .font(.footnote)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .foregroundStyle(Color.red)
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(Color.red.opacity(0.10))
        )
        .transition(.opacity.combined(with: .move(edge: .top)))
    }

    /// Drawn rather than left to `.borderedProminent`, whose disabled state on
    /// a bar background fades to grey text with no button under it — the reader
    /// cannot tell a not-yet-fillable form from a missing control.
    private var pairBar: some View {
        Button {
            focus = nil
            Task { await pair() }
        } label: {
            HStack(spacing: 8) {
                if isPairing {
                    ProgressView().controlSize(.small).tint(.white)
                }
                Text(isPairing ? "Pairing…" : "Pair")
                    .font(.body.weight(.semibold))
            }
            .foregroundStyle(.white)
            .frame(maxWidth: .infinity)
            .frame(height: 50)
            .background(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(Theme.ink.opacity(canPair && !isPairing ? 1 : 0.30))
            )
        }
        .buttonStyle(.plain)
        .disabled(isPairing || !canPair)
        .animation(.easeOut(duration: 0.2), value: canPair)
        .padding(.horizontal, Theme.gutter)
        .padding(.top, 10)
        .padding(.bottom, 8)
        .background(.bar)
        .overlay(alignment: .top) { Divider().opacity(0.6) }
    }

    // MARK: Behaviour

    private func pair() async {
        isPairing = true
        errorMessage = nil
        defer { isPairing = false }
        do {
            try await model.pair(baseURL: baseURL, code: code, label: label)
            Haptic.success()
            dismiss()
        } catch {
            Haptic.failure()
            errorMessage = error.localizedDescription
        }
    }

    private enum Field: Hashable {
        case address
        case code
        case label
    }
}

/// A labelled field row. The label sits above the value rather than beside it,
/// so a long address is never squeezed into half the width.
private struct FieldRow<Content: View>: View {
    let symbol: String
    let title: String
    var optional = false
    @ViewBuilder var content: Content

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(Theme.ink)
                .frame(width: 24)

            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    Text(title)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    if optional {
                        Text("optional")
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                    }
                }
                content
                    .font(.body)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }
}
