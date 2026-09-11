import SwiftUI

/// Adding a remote agent, name-first.
///
/// **The name leads because it is the only path that works from anywhere.** A QR
/// scan only helps when you are standing at the desktop showing the code — and if
/// you are standing there, you are standing where the Approve button is anyway. So
/// the hero is a single field, the agent's name; what it costs the person on the
/// other end is one tap.
///
/// Typing a full address and a one-time code is still here, folded away, because a
/// self-hosted agent has no name in the default zone — and because the stage's
/// "Add again" arrives with an address already filled in.
struct AddAgentView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: AppModel
    let request: AddAgentRequest

    @State private var stage = Stage.naming
    @State private var name = ""
    @State private var baseURL: String
    @State private var code: String
    @State private var label: String
    @State private var errorMessage: String?
    @State private var isWorking = false
    @State private var showingScanner = false
    @State private var showingAddress: Bool
    @State private var invitation: JoinInvitation?
    @FocusState private var focus: Field?

    init(request: AddAgentRequest) {
        self.request = request
        _baseURL = State(initialValue: request.baseURL)
        _code = State(initialValue: request.code)
        _label = State(initialValue: request.label)
        // An "Add again" from the stage arrives knowing the address, so open on the
        // card that holds it rather than on a name field it already has the answer to.
        _showingAddress = State(initialValue: !request.baseURL.isEmpty)
    }

    var body: some View {
        NavigationStack {
            Group {
                switch stage {
                case .naming:
                    naming
                case .waiting:
                    waiting
                }
            }
            .hiCanvas()
            .navigationTitle("Add a remote hi-agent")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
            .fullScreenCover(isPresented: $showingScanner) {
                AgentQRScannerView { scanned in
                    baseURL = scanned.baseURL
                    code = scanned.code
                    if !scanned.label.isEmpty {
                        label = scanned.label
                    }
                    showingAddress = true
                    errorMessage = nil
                    Haptic.tap()
                }
            }
            .onChange(of: request.id) { _, _ in
                baseURL = request.baseURL
                code = request.code
                label = request.label
                showingAddress = !request.baseURL.isEmpty
                errorMessage = nil
                stage = .naming
            }
            .task {
                guard request.opensScanner else { return }
                showingScanner = true
            }
        }
    }

    // MARK: Naming

    private var naming: some View {
        ScrollView {
            VStack(spacing: 20) {
                hero
                nameField
                askButton
                scanLink
                addressDisclosure
                if let errorMessage {
                    errorBanner(errorMessage)
                }
            }
            .padding(.horizontal, Theme.gutter)
            .padding(.top, 8)
            .padding(.bottom, 32)
            .hiMeasure()
            .animation(.smooth(duration: 0.25), value: errorMessage)
            .animation(.smooth(duration: 0.25), value: showingAddress)
        }
        .scrollDismissesKeyboard(.interactively)
    }

    private var hero: some View {
        VStack(spacing: 14) {
            CoreMark(size: 84)
            Text("Say which agent this is. It will ask to be let in, and you approve it there.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 320)
        }
        .padding(.top, 6)
    }

    /// The address drawn as one line — the box holds only the part that varies, and
    /// the zone sits beside it in the same type. Typing `iloahz` and seeing
    /// `iloahz.hi-agent.xyz` assemble itself is the whole explanation of what a name
    /// is, with no sentence spent on it.
    /// One face for the whole address, in both toolkits' vocabularies, so the
    /// measurement and the thing measured cannot drift apart. Sized off the body
    /// text style rather than a literal, so Dynamic Type still moves it.
    private var namePointSize: CGFloat {
        UIFont.preferredFont(forTextStyle: .body).pointSize
    }

    private var nameFont: Font {
        .system(size: namePointSize, weight: .semibold, design: .monospaced)
    }

    private var nameUIFont: UIFont {
        .monospacedSystemFont(ofSize: namePointSize, weight: .semibold)
    }

    /// How wide the name box has to be to hold exactly what is in it — the
    /// placeholder when it is empty. Plus two points, so the caret at the end of the
    /// text has somewhere to stand.
    private var nameWidth: CGFloat {
        let shown = name.isEmpty ? "your agent" : name
        return ceil((shown as NSString).size(withAttributes: [.font: nameUIFont]).width) + 2
    }

    private var nameField: some View {
        Card(padding: 16) {
            // The field hugs its own text so the zone sits *against* it: typing
            // `iloahz` assembles `iloahz.hi-agent.xyz` in one line. Left to fill the
            // width it pushes the zone to the far margin, and the two read as a box
            // and an unrelated label rather than as one address.
            //
            // Given an explicit measured width rather than `fixedSize` or a ghost
            // behind it: a `TextField` claims every point offered in both of those,
            // and the gap it leaves is the thing this layout exists to close. A fixed
            // frame also means an over-long name truncates the *zone* rather than
            // pushing what you typed off the card — the right way round.
            HStack(spacing: 0) {
                TextField("your agent", text: $name)
                    .font(nameFont)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.URL)
                    .submitLabel(.go)
                    .focused($focus, equals: .name)
                    .onSubmit { Task { await ask() } }
                    .frame(width: nameWidth)

                Text(".\(CoreClient.defaultZone)")
                    .font(nameFont)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity)
            // The whole card is the target — a caret-width box is not something to
            // ask a thumb to find.
            .contentShape(Rectangle())
            .onTapGesture { focus = .name }
        }
    }

    /// Drawn rather than left to `.borderedProminent`, whose disabled state fades to
    /// grey text with no button under it — the reader cannot tell a not-yet-fillable
    /// form from a missing control.
    ///
    /// The disabled state keeps the shape and drops to a wash with a **secondary**
    /// label, rather than white on a dimmed fill: white on 30%-ink is the same
    /// unreadable-control failure by another route, and this is the state the screen
    /// *opens* in, so it is the first thing anybody sees.
    private var askButton: some View {
        let live = canAsk && !isWorking
        return Button {
            focus = nil
            Task { await ask() }
        } label: {
            HStack(spacing: 8) {
                if isWorking {
                    ProgressView().controlSize(.small).tint(.white)
                }
                Text(isWorking ? "Asking…" : "Ask to be let in")
                    .font(.body.weight(.semibold))
            }
            .foregroundStyle(live ? AnyShapeStyle(.white) : AnyShapeStyle(.secondary))
            .frame(maxWidth: .infinity)
            .frame(height: 50)
            .background(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(Theme.ink.opacity(live ? 1 : 0.12))
            )
        }
        .buttonStyle(.plain)
        .disabled(isWorking || !canAsk)
        .animation(.easeOut(duration: 0.2), value: live)
    }

    private var scanLink: some View {
        Button {
            focus = nil
            showingScanner = true
        } label: {
            Label("Scan a QR code instead", systemImage: "qrcode.viewfinder")
                .font(.subheadline.weight(.medium))
        }
        .tint(Theme.ink)
    }

    /// The old form, folded away. Everything a self-hosted agent needs and nobody
    /// else does.
    private var addressDisclosure: some View {
        VStack(spacing: 12) {
            Button {
                focus = nil
                withAnimation(.smooth(duration: 0.25)) { showingAddress.toggle() }
            } label: {
                HStack(spacing: 6) {
                    Text("Use a full address")
                    Image(systemName: "chevron.down")
                        .font(.system(size: 11, weight: .bold))
                        .rotationEffect(.degrees(showingAddress ? 0 : -90))
                }
                .font(.subheadline.weight(.medium))
            }
            .tint(.secondary)

            if showingAddress {
                fields
                Button {
                    focus = nil
                    Task { await add() }
                } label: {
                    HStack(spacing: 8) {
                        if isWorking {
                            ProgressView().controlSize(.small)
                        }
                        Text("Add")
                            .font(.body.weight(.semibold))
                    }
                    .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .controlSize(.large)
                .buttonBorderShape(.roundedRectangle(radius: 14))
                .tint(Theme.ink)
                .disabled(isWorking || !canAdd)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .padding(.top, 4)
    }

    private var fields: some View {
        Card(padding: 0) {
            VStack(spacing: 0) {
                FieldRow(symbol: "link", title: "Address") {
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

                FieldRow(symbol: "key", title: "One-time code") {
                    TextField("the code it is showing", text: $code)
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

    // MARK: Waiting

    /// One number, big, and the sentence that says who has to act. Nothing else —
    /// there is nothing for this person to do but hold the phone up.
    private var waiting: some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)

            Card(padding: 28) {
                VStack(spacing: 16) {
                    Text(invitation?.code ?? "")
                        .font(.system(size: 54, weight: .bold, design: .monospaced))
                        .kerning(6)
                        .contentTransition(.numericText())
                        .minimumScaleFactor(0.6)
                        .lineLimit(1)

                    Text("Approve this on \(invitation?.label ?? "your agent") to finish.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)

                    ProgressView()
                        .controlSize(.small)
                        .padding(.top, 2)

                    if let errorMessage {
                        Text(errorMessage)
                            .font(.footnote)
                            .foregroundStyle(Color.red)
                            .multilineTextAlignment(.center)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: .infinity)
            }
            .padding(.horizontal, Theme.gutter)
            .hiMeasure()

            Spacer(minLength: 0)

            Button("Cancel") { stopWaiting() }
                .font(.subheadline.weight(.medium))
                .tint(Theme.ink)
                .padding(.bottom, 24)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // The wait belongs to this screen: leaving it cancels the poll, and the
        // request expires on its own at the agent.
        .task(id: invitation) {
            guard let invitation else { return }
            do {
                try await model.join(invitation)
                Haptic.success()
                dismiss()
            } catch is CancellationError {
                return
            } catch {
                Haptic.failure()
                errorMessage = error.localizedDescription
                stage = .naming
                self.invitation = nil
            }
        }
    }

    // MARK: Pieces

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

    // MARK: Behaviour

    private var canAsk: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var canAdd: Bool {
        !baseURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func ask() async {
        guard canAsk, !isWorking else { return }
        isWorking = true
        errorMessage = nil
        defer { isWorking = false }
        do {
            let asked = try await model.askToJoin(name: name)
            Haptic.tap()
            invitation = asked
            stage = .waiting
        } catch {
            Haptic.failure()
            errorMessage = error.localizedDescription
        }
    }

    private func stopWaiting() {
        // Clearing the invitation cancels the `.task` keyed on it, which is the
        // poll — there is no separate handle to tear down.
        invitation = nil
        stage = .naming
    }

    private func add() async {
        guard canAdd, !isWorking else { return }
        isWorking = true
        errorMessage = nil
        defer { isWorking = false }
        do {
            try await model.add(baseURL: baseURL, code: code, label: label)
            Haptic.success()
            dismiss()
        } catch {
            Haptic.failure()
            errorMessage = error.localizedDescription
        }
    }

    private enum Stage {
        case naming
        case waiting
    }

    private enum Field: Hashable {
        case name
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
