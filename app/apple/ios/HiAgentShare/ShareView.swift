import SwiftUI

/// The share sheet's whole interface, which is one line and two buttons.
///
/// **It exists for the button, not for the information.** This extension has one
/// destination and nothing to ask — no album, no recipient, no folder — so it was
/// originally built with no interface at all: copy the bytes, try to open the app,
/// disappear. The open is what forced a screen. `EnvironmentValues().openURL` is the
/// only call that still wakes a containing app ([`OpenHost`]), and the shape it is
/// known to work in is the one every app that does this uses: a screen, a tap, and
/// the dismiss going out just ahead of the open.
///
/// So the tap is the mechanism, and the screen is what the tap needs to live on.
/// Everything else here is kept out: no note field (what was shared is what was
/// said, and the conversation is one tap away), no per-file preview, no agent
/// picker. What it says is what a person needs to decide between the two buttons —
/// how much is going, and to stay here or go talk about it.
struct ShareView: View {
    @ObservedObject var model: ShareModel
    let onDone: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            Spacer(minLength: 0)
            content
            Spacer(minLength: 0)
            actions
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .animation(.smooth(duration: 0.25), value: model.state)
    }

    @ViewBuilder
    private var content: some View {
        switch model.state {
        case .staging:
            VStack(spacing: 14) {
                ProgressView()
                Text("Taking it…")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }

        case .ready(let count):
            VStack(spacing: 10) {
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 38))
                    .foregroundStyle(.green)
                Text(count > 1 ? "\(count) things ready for your agent" : "Ready for your agent")
                    .font(.headline)
                    .multilineTextAlignment(.center)
                Text("It sends as soon as Hi Agent is open.")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }

        case .failed(let reason):
            VStack(spacing: 10) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 34))
                    .foregroundStyle(.orange)
                Text(reason)
                    .font(.subheadline)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    @ViewBuilder
    private var actions: some View {
        switch model.state {
        case .staging:
            Button("Cancel", action: onDone)
                .font(.subheadline)
                .tint(.secondary)

        case .ready:
            VStack(spacing: 12) {
                // The primary, because it is the reason to share something to your
                // agent: you want to say a thing about it, and that happens in the
                // conversation.
                Button {
                    model.openApp(then: onDone)
                } label: {
                    Text("Open and say something")
                        .fontWeight(.semibold)
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)

                // Not a cancel: what was shared is already on disk either way, and
                // this is the "I'll get to it" that leaves you where you were.
                Button("Not now", action: onDone)
                    .font(.subheadline)
            }

        case .failed:
            Button("Close", action: onDone)
                .font(.subheadline.weight(.semibold))
        }
    }
}
