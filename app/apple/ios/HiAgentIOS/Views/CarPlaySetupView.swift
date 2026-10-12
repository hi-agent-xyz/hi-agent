import AVFoundation
import SwiftUI

struct CarPlaySetupView: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var phase
    @State private var permission = AVAudioApplication.shared.recordPermission
    @State private var requesting = false

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Text("Pair your agent on iPhone, then open Hi Agent in CarPlay and choose the agent to start talking.")
                    Text("CarPlay voice conversations require iOS 26.4 or later and a build with Apple's CarPlay permission enabled.")
                        .foregroundStyle(.secondary)
                }
                Section {
                    if permission == .granted {
                        Label("Microphone access enabled", systemImage: "checkmark.circle")
                    } else if permission == .denied {
                        Button("Open Settings to enable microphone") {
                            if let url = URL(string: UIApplication.openSettingsURLString) {
                                UIApplication.shared.open(url)
                            }
                        }
                    } else {
                        Button(requesting ? "Requesting access…" : "Enable microphone") {
                            requesting = true
                            AVAudioApplication.requestRecordPermission { _ in
                                Task { @MainActor in
                                    permission = AVAudioApplication.shared.recordPermission
                                    requesting = false
                                }
                            }
                        }
                        .disabled(requesting)
                    }
                } header: {
                    Text("Microphone")
                } footer: {
                    Text("Complete setup when parked. Recording starts only when you start a conversation and stops when you end it or leave Hi Agent in CarPlay.")
                }
            }
            .navigationTitle("CarPlay")
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
            .onChange(of: phase) { _, value in
                if value == .active { permission = AVAudioApplication.shared.recordPermission }
            }
        }
    }
}

/// Removing the web stage during native capture avoids two microphone uploaders
/// and two playback subscribers competing for the same audio session.
struct CarPlayPhoneView: View {
    @ObservedObject var session: CarPlayVoiceSession

    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "car.fill").font(.largeTitle).accessibilityHidden(true)
            Text(session.state == .failed ? "CarPlay conversation stopped" : "Talking in CarPlay").font(.title2)
            Text(session.state == .failed ? session.message : "Use your car's controls to end the conversation.")
                .foregroundStyle(.secondary)
            Button("End conversation") { session.stop() }.buttonStyle(.borderedProminent)
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .hiCanvas()
    }
}
