# CarPlay entitlement application

Prepared on 2026-10-12. Status: draft, not submitted. An initial CarPlay client is
implemented; runtime and vehicle validation remain pending. The text below is prepared for copying into Apple's application;
field names and category availability must be checked in the authenticated form.

## App details

- App name: Hi Agent
- Website: https://hi-agent.xyz
- iOS bundle identifier in the repository: `com.xiaoyuanzhu.hiagent.ios`
- Requested category: Voice-based conversational apps

Confirm the actual registered bundle identifier in the developer account before
submission: the release workflow can override the repository default. Obtain the
Team ID and applicant contact details from that account. Do not substitute an
audio or communication entitlement if the requested category is unavailable.

## Application copy (English)

### App description

Hi Agent is a personal AI agent client for iPhone and iPad. It connects users to
their paired remote agent, allowing them to converse with the agent and receive
assistance. Pairing and credentials are managed by the native iOS application.
The existing iOS client presents the agent's interface in an authenticated web
view. We are requesting CarPlay support for a dedicated native, voice-first
experience, rather than mirroring this web interface onto the vehicle display.

### Proposed CarPlay experience

We would like to support the voice-based conversational app category. A user
would select Hi Agent from the CarPlay Home screen and start a spoken conversation
with an agent previously paired on their iPhone. The user could ask questions,
discuss ideas, or dictate thoughts, and receive spoken responses without reading
a transcript or typing on the vehicle display.

The proposed interface would use the CarPlay framework templates available to
this category. It would provide minimal controls to start and end a conversation,
select an already paired agent where permitted, and indicate connection and
voice-session status. If a pairing or configuration step is needed, the CarPlay
interface would indicate that setup must be completed on iPhone when safe.

### Driver interaction and audio

The proposed driving experience is centered on audio input and spoken output.
It would not expose the full chat history, a keyboard, file browsing, camera
capture, or complex agent configuration on the vehicle display. Pairing,
authentication, and configuration would be completed on iPhone before use.

We plan to implement native audio capture and playback, microphone permission
handling, audio interruptions, and clear user controls for ending a session.
Network failures would be represented with a concise status and a simple retry
action. We would validate the experience in the CarPlay Simulator and on a
physical vehicle before submitting the CarPlay-enabled build for review.

### Current development status

The iOS client and remote agent connection already exist. An initial dedicated
CarPlay scene and native audio implementation are now in development, with a
paired-agent list, voice-control screen, continuous microphone upload, and streamed
MP3 playback. Vehicle validation is pending. We are requesting the appropriate
entitlement before completing device testing and distribution. We can provide the
interaction flow and, after runtime validation, a demonstration build or recording.

## Proposed interaction flow (not an implemented screen)

1. Before driving: pair the agent and grant microphone access on iPhone.
2. Open Hi Agent from the CarPlay Home screen.
3. See the current paired agent and a start-conversation action.
4. Start a voice session; show a minimal listening/responding indicator while
   requests and responses are exchanged through audio.
5. End the session using an always-accessible end action.
6. If offline, show connection status and a retry action. If setup is incomplete,
   indicate that setup is required on iPhone when safe.

Final controls and templates depend on the approved category's requirements.

## Submission and next steps

1. Sign into https://developer.apple.com/contact/request/carplay/ using the
   account that owns the iOS App ID.
2. Verify applicant details, Team ID, registered bundle identifier, and the
   voice-based conversational category offered in the form.
3. Adapt the application copy to the actual form fields. Review and agree to the
   CarPlay Entitlement Addendum as the authorized account holder.
4. Submit and record Apple's request/reference number and any follow-up here.
5. On approval, enable the granted capability for the App ID and update signing
   profiles and project entitlements using Apple's supplied identifier.
6. Verify the dedicated CarPlay scene and native voice transport, including
   foreground/locked-phone behavior, interruptions, connection loss, and end-session
   controls on simulator and physical hardware.

Apple publishes `com.apple.developer.carplay-voice-based-conversation` for iOS 26.4+.
An opt-in entitlement file is included with the implementation; the default
shipping signing configuration does not claim the capability before approval.

## Official references

- Categories: https://developer.apple.com/carplay/
- Application: https://developer.apple.com/contact/request/carplay/
- Approval and signing: https://developer.apple.com/documentation/carplay/requesting-carplay-entitlements
- Developer guide: https://developer.apple.com/download/files/CarPlay-Developer-Guide.pdf
- Design guidelines: https://developer.apple.com/design/human-interface-guidelines/carplay
