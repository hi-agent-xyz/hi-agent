import SwiftUI

/// The core switcher. A place you drop into from the stage and leave again —
/// so it is a sheet with a medium detent, not a screen you have to navigate
/// back out of.
struct RosterView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var model: AppModel

    var body: some View {
        NavigationStack {
            List {
                Section {
                    ForEach(model.entries) { entry in
                        Button {
                            select(entry)
                        } label: {
                            CoreRow(entry: entry)
                        }
                        .buttonStyle(.plain)
                        .listRowInsets(EdgeInsets(top: 10, leading: 16, bottom: 10, trailing: 16))
                        .swipeActions(edge: .trailing) {
                            Button(role: .destructive) {
                                forget(entry)
                            } label: {
                                Label("Forget", systemImage: "trash")
                            }
                        }
                        .contextMenu {
                            if !entry.attached {
                                Button {
                                    select(entry)
                                } label: {
                                    Label("Use this core", systemImage: "checkmark.circle")
                                }
                            }
                            Button(role: .destructive) {
                                forget(entry)
                            } label: {
                                Label("Forget", systemImage: "trash")
                            }
                        }
                    }
                } footer: {
                    Text("Pull down to re-check whether each core is answering.")
                        .font(.footnote)
                }
            }
            .listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .hiCanvas()
            .refreshable {
                await model.refresh()
            }
            .navigationTitle("Cores")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                        .font(.body.weight(.semibold))
                }
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        model.pairingRequest = .scan
                        dismiss()
                    } label: {
                        Image(systemName: "plus")
                    }
                    .accessibilityLabel("Pair a core")
                }
            }
            .safeAreaInset(edge: .bottom) {
                Button {
                    model.pairingRequest = .scan
                    dismiss()
                } label: {
                    Label("Pair another core", systemImage: "qrcode.viewfinder")
                        .font(.body.weight(.semibold))
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .buttonBorderShape(.roundedRectangle(radius: 14))
                .padding(.horizontal, 16)
                .padding(.top, 10)
                .padding(.bottom, 8)
                .background(.bar)
            }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .presentationBackground(.regularMaterial)
        .presentationCornerRadius(28)
    }

    private func select(_ entry: RosterEntry) {
        guard !entry.attached else {
            dismiss()
            return
        }
        Haptic.tap()
        model.attach(entry.id)
        dismiss()
    }

    private func forget(_ entry: RosterEntry) {
        try? model.forget(entry.id)
    }
}

/// One core. Name, address, and — only when it is worth saying — what the last
/// health check found. A healthy roster stays quiet.
private struct CoreRow: View {
    let entry: RosterEntry

    private var host: String {
        guard let url = URL(string: entry.baseURL), let host = url.host else {
            return entry.baseURL
        }
        let path = url.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        let port = url.port.map { ":\($0)" } ?? ""
        return path.isEmpty ? "\(host)\(port)" : "\(host)\(port)/\(path)"
    }

    var body: some View {
        HStack(spacing: 12) {
            StatusDot(state: entry.health)

            VStack(alignment: .leading, spacing: 2) {
                Text(entry.label)
                    .font(Theme.display(17, .semibold))
                    .foregroundStyle(.primary)
                    .lineLimit(1)

                HStack(spacing: 6) {
                    Text(host)
                        .lineLimit(1)
                        .truncationMode(.middle)

                    if !entry.health.isLive {
                        Text("·")
                        Text(entry.health.title)
                    }
                }
                .font(.footnote)
                .foregroundStyle(.secondary)
            }

            Spacer(minLength: 8)

            if entry.attached {
                Image(systemName: "checkmark")
                    .font(.system(size: 14, weight: .bold))
                    .foregroundStyle(Theme.ink)
                    .accessibilityHidden(true)
            }
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            "\(entry.label), \(entry.health.title)\(entry.attached ? ", current core" : "")"
        )
    }
}
