import SwiftUI

@main
struct HiAgentIOSApp: App {
    @StateObject private var model = AppModel()
    @StateObject private var network = NetworkMonitor()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(model)
                .environmentObject(network)
        }
    }
}
