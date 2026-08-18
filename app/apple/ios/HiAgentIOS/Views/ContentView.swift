import SwiftUI

struct ContentView: View {
    @EnvironmentObject private var model: AppModel
    @State private var showingPairSheet = false

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
                        showingPairSheet = true
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
        .sheet(isPresented: $showingPairSheet) {
            PairCoreView()
                .environmentObject(model)
        }
        .task {
            await model.refresh()
            if model.entries.isEmpty {
                showingPairSheet = true
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
    @EnvironmentObject private var model: AppModel
    let entry: RosterEntry
    @State private var session: CoreSession?
    @State private var errorMessage: String?
    @State private var isLoading = false

    var body: some View {
        Group {
            if let session {
                CoreWebView(session: session)
                    .ignoresSafeArea(edges: .bottom)
            } else if isLoading {
                ProgressView("Opening \(entry.label)…")
            } else {
                ContentUnavailableView(
                    "Could not open this core",
                    systemImage: "wifi.exclamationmark",
                    description: Text(errorMessage ?? "Try opening it again.")
                )
                Button("Retry") {
                    Task { await open() }
                }
                .buttonStyle(.borderedProminent)
            }
        }
        .navigationTitle(entry.label)
        .navigationBarTitleDisplayMode(.inline)
        .task(id: entry.id) {
            await open()
        }
    }

    private func open() async {
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }
        do {
            session = try await model.open(entry.id)
        } catch {
            session = nil
            errorMessage = error.localizedDescription
        }
    }
}

private struct PairCoreView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: AppModel
    @State private var baseURL = ""
    @State private var code = ""
    @State private var label = ""
    @State private var errorMessage: String?
    @State private var isPairing = false

    var body: some View {
        NavigationStack {
            Form {
                Section {
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
