import SwiftUI

@main
struct HiAgentIOSApp: App {
    // The shared instance rather than a fresh one: `ShowScreenIntent` runs in this
    // process without the environment, and both must see one roster.
    @StateObject private var model = AppModel.shared
    @StateObject private var network = NetworkMonitor()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(model)
                .environmentObject(network)
        }
    }
}
