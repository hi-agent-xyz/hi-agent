import AppIntents
import UniformTypeIdentifiers

/// The action a shortcut calls to hand the agent the screen — the phone's half of
/// "come and see this" (see [`ShowScreen`]).
///
/// It takes the image rather than producing one, because the ordering is the whole
/// point: the picture must already exist before Hi Agent comes to the front, or the
/// only thing on screen to photograph is Hi Agent. So the shortcut is
/// `Take Screenshot → Show My Screen`, and this runs second.
///
/// `openAppWhenRun` is what makes the gesture feel like the desktop's: the app is
/// brought forward and `perform()` runs inside it, so the send has the Keychain, the
/// roster, and a screen to report on. It never throws — a failure here would surface
/// as a Shortcuts error banner over the app the person was using, while the app it
/// just opened, holding the bytes and able to retry, said nothing. Everything that
/// can go wrong is reported through `AppModel.showScreenState` instead.
struct ShowScreenIntent: AppIntent {
    static var title: LocalizedStringResource = "Show My Screen"

    static var description = IntentDescription(
        """
        Hands a screenshot to the core this device is attached to, and opens Hi Agent \
        on the conversation. Put a Take Screenshot action ahead of this one and give \
        it to the Action Button.
        """
    )

    static var openAppWhenRun: Bool = true

    // `supportedTypeIdentifiers` rather than the `supportedContentTypes: [UTType]`
    // spelling of the same thing: that overload is iOS 18, and this target still
    // deploys to 17.
    @Parameter(
        title: "Screen",
        description: "The screenshot to hand over.",
        supportedTypeIdentifiers: ["public.image"]
    )
    var screen: IntentFile

    @Parameter(
        title: "Note",
        description: "What you're saying as you hand it over. Left empty, the agent is told this is your screen right now.",
        default: nil
    )
    var note: String?

    static var parameterSummary: some ParameterSummary {
        Summary("Show \(\.$screen) to Hi Agent") {
            \.$note
        }
    }

    @MainActor
    func perform() async throws -> some IntentResult {
        let said = note?.trimmingCharacters(in: .whitespacesAndNewlines)
        await AppModel.shared.showScreen(
            data: screen.data,
            type: screen.type,
            note: (said?.isEmpty == false) ? said : nil
        )
        return .result()
    }
}
