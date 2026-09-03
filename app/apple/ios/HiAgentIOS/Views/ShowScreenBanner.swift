import SwiftUI

/// What happened to the screen you just showed.
///
/// The gesture happens somewhere else — you press the Action Button inside another
/// app — and its whole result is a message in a conversation you are not looking at
/// yet. By the time Hi Agent is on screen the face may still be loading, so without
/// this the successful case and the "not paired with anything" case look identical:
/// the app opened, and nothing visibly happened.
///
/// It says its piece and goes. Only a failure stays, because only a failure is
/// something to act on.
struct ShowScreenBanner: View {
    let state: ShowScreenState?
    let onRetry: () -> Void
    let onDismiss: () -> Void

    /// How long a landed screen keeps saying so. Long enough to read six words while
    /// the face is still painting.
    private static let sentDwell: Duration = .seconds(2.4)

    var body: some View {
        Group {
            if let state {
                content(for: state)
                    .padding(.horizontal, 14)
                    .hiMeasure()
                    .padding(.bottom, 14)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.smooth(duration: 0.3), value: state)
        // Keyed on the state so a second screen shown while the first banner is up
        // restarts the dwell instead of inheriting the old countdown.
        .task(id: state) {
            switch state {
            case .sent:
                Haptic.success()
                try? await Task.sleep(for: Self.sentDwell)
                guard !Task.isCancelled else {
                    return
                }
                onDismiss()
            case .failed:
                Haptic.failure()
            case .sending, .none:
                break
            }
        }
    }

    @ViewBuilder
    private func content(for state: ShowScreenState) -> some View {
        switch state {
        case .sending:
            row {
                ProgressView()
                    .controlSize(.small)
                Text("Showing your screen…")
                    .font(.subheadline.weight(.medium))
                Spacer(minLength: 0)
            }

        case .sent(let coreLabel):
            row {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                Text("Shown to \(coreLabel)")
                    .font(.subheadline.weight(.medium))
                    .lineLimit(1)
                Spacer(minLength: 0)
            }

        case .failed(let reason):
            VStack(alignment: .leading, spacing: 12) {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                    Text(reason)
                        .font(.subheadline)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                }
                HStack(spacing: 12) {
                    Button("Try again", action: onRetry)
                        .font(.subheadline.weight(.semibold))
                    Spacer(minLength: 0)
                    Button("Dismiss", action: onDismiss)
                        .font(.subheadline)
                        .tint(.secondary)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .strokeBorder(Theme.hairline, lineWidth: 1)
            )
            .shadow(color: .black.opacity(0.14), radius: 12, y: 4)
        }
    }

    private func row<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        HStack(spacing: 10) {
            content()
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .background(.regularMaterial, in: Capsule())
        .overlay(Capsule().strokeBorder(Theme.hairline, lineWidth: 1))
        .shadow(color: .black.opacity(0.14), radius: 12, y: 4)
    }
}
