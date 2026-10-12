import AVFoundation
import CarPlay
import Combine

@MainActor
final class CarPlaySceneDelegate: UIResponder, @preconcurrency CPTemplateApplicationSceneDelegate, CPInterfaceControllerDelegate {
    private var controller: CPInterfaceController?
    private var roster: CPListTemplate?
    private var voice: CPVoiceControlTemplate?
    private var subscriptions = Set<AnyCancellable>()
    private let session = CarPlayVoiceSession.shared
    private var connectionID = UUID()

    func templateApplicationScene(_ scene: CPTemplateApplicationScene, didConnect controller: CPInterfaceController) {
        connectionID = UUID()
        self.controller = controller
        controller.delegate = self
        let roster = CPListTemplate(title: "Hi Agent", sections: [])
        self.roster = roster
        updateRoster()
        controller.setRootTemplate(roster, animated: false, completion: nil)
        AppModel.shared.$entries.receive(on: RunLoop.main).sink { [weak self] _ in
            guard let self else { return }
            if let id = self.session.entryID, AppModel.shared.entry(id: id) == nil {
                self.end()
            }
            self.updateRoster()
        }.store(in: &subscriptions)
        session.$state.receive(on: RunLoop.main).sink { [weak self] state in
            self?.updateVoice(state)
        }.store(in: &subscriptions)
    }

    func templateApplicationScene(_ scene: CPTemplateApplicationScene, didDisconnectInterfaceController controller: CPInterfaceController) {
        session.stop()
        connectionID = UUID()
        subscriptions.removeAll()
        controller.delegate = nil
        self.controller = nil
        roster = nil
        voice = nil
    }

    func sceneDidEnterBackground(_ scene: UIScene) {
        // Switching to another CarPlay app ends capture. Locking the phone alone
        // does not background the separate CarPlay scene.
        session.stop()
        voice = nil
        if let roster { controller?.setRootTemplate(roster, animated: false, completion: nil) }
    }

    func templateDidDisappear(_ template: CPTemplate, animated: Bool) {
        if template === voice {
            session.stop()
            voice = nil
        }
    }

    private func updateRoster() {
        let entries = AppModel.shared.entries
        let items = entries.map { entry in
            let item = CPListItem(text: entry.label, detailText: "Start voice conversation", image: UIImage(systemName: "mic.fill"))
            item.handler = { [weak self] _, completion in
                self?.start(entry)
                completion()
            }
            return item
        }
        if items.isEmpty {
            let setup = CPListItem(text: "Set up on iPhone", detailText: "Pair an agent in Hi Agent when parked.")
            setup.isEnabled = false
            roster?.updateSections([CPListSection(items: [setup])])
        } else {
            roster?.updateSections([CPListSection(items: items)])
        }
    }

    private func start(_ entry: RosterEntry) {
        guard let controller, voice == nil else { return }
        guard #available(iOS 26.4, *) else {
            showAlert("CarPlay voice conversations require iOS 26.4 or later.")
            return
        }
        guard AVAudioApplication.shared.recordPermission == .granted else {
            showAlert("Enable microphone access in Hi Agent → Agents → CarPlay on iPhone when parked.")
            return
        }
        let states = [
            CPVoiceControlState(identifier: "connecting", titleVariants: ["Connecting to \(entry.label)", "Connecting"], image: UIImage(systemName: "ellipsis"), repeats: false),
            CPVoiceControlState(identifier: "listening", titleVariants: ["\(entry.label) is listening", "Listening"], image: UIImage(systemName: "mic.fill"), repeats: false),
            CPVoiceControlState(identifier: "speaking", titleVariants: ["\(entry.label) is speaking", "Speaking"], image: UIImage(systemName: "speaker.wave.2.fill"), repeats: false),
            CPVoiceControlState(identifier: "failed", titleVariants: ["Conversation stopped. Tap Retry.", "Tap Retry"], image: UIImage(systemName: "wifi.exclamationmark"), repeats: false),
        ]
        let voice = CPVoiceControlTemplate(voiceControlStates: states)
        // CPVoiceControlTemplate adopts this public protocol on iOS 26.4. Using
        // the protocol permits compilation with the project's older SDK without
        // private selectors, while requiring the supported runtime before use.
        guard let controls = voice as? CPBarButtonProviding else {
            showAlert("Update iOS to use CarPlay voice conversations.")
            return
        }
        controls.trailingNavigationBarButtons = [CPBarButton(title: "End") { [weak self] _ in
            self?.end()
        }]
        controls.leadingNavigationBarButtons = [CPBarButton(title: "Retry") { [weak self] _ in
            guard let self, self.session.state == .failed else { return }
            self.session.start(entry: entry)
        }]
        self.voice = voice
        let token = connectionID
        controller.pushTemplate(voice, animated: true) { [weak self] success, _ in
            guard let self, self.connectionID == token, self.voice === voice else { return }
            guard success else { self.voice = nil; return }
            // The required voice screen is visible before activating recording.
            self.session.start(entry: entry)
        }
    }

    private func updateVoice(_ state: CarPlayVoiceSession.State) {
        guard let voice, state == session.state else { return }
        if state == .idle {
            end()
            return
        }
        voice.activateVoiceControlState(withIdentifier: state.rawValue)
        if let controls = voice as? CPBarButtonProviding {
            controls.leadingNavigationBarButtons.first?.isEnabled = state == .failed
        }
    }

    private func end() {
        voice = nil
        session.stop()
        controller?.popToRootTemplate(animated: true, completion: nil)
    }

    private func showAlert(_ title: String) {
        let alert = CPAlertTemplate(titleVariants: [title], actions: [CPAlertAction(title: "OK", style: .default) { [weak self] _ in
            self?.controller?.dismissTemplate(animated: true, completion: nil)
        }])
        controller?.presentTemplate(alert, animated: true, completion: nil)
    }
}
