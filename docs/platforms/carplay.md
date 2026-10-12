# CarPlay voice client

The iOS app includes a CarPlay scene, a paired-agent list, and a native voice
session. Choose an agent to present the voice-control screen before opening the
microphone. End, system Back, leaving the CarPlay app, or disconnecting the car
ends the session. Audio interruptions and route changes end capture with a Retry
state; recording does not resume automatically after a phone call.

## Platform and authorization

Voice-based conversational apps require iOS 26.4 or later. The app still deploys
to iOS 17; CarPlay voice start is guarded at runtime. The scene uses only the
list, alert, and voice-control templates supported by this category. Navigation
buttons on the voice-control template are accessed via the public
`CPBarButtonProviding` protocol, whose runtime adoption begins in iOS 26.4.

Apple's published entitlement is
`com.apple.developer.carplay-voice-based-conversation`. Request it using the
[application draft](carplay-application.md). The default shipping entitlements
remain usable before Apple grants this capability. Once approved, enable the
capability for the registered App ID, regenerate profiles, and set **the app
target's** Code Signing Entitlements to `HiAgentIOS/CarPlay.entitlements` in Xcode.
Do not override that setting across all targets: the share extension needs its
own entitlements. The opt-in file includes the app's existing associated domain
and App Group capabilities as well as CarPlay; keep those in sync when changed.

No application has been submitted and no approval is assumed.

## Audio and session ownership

- Authentication reuses `AppModel.open` and its short-lived `hi_surface` cookie.
  The cookie is explicitly attached to an ephemeral native URLSession's requests.
- Microphone input uses AVAudioEngine voice processing and AVAudioConverter to
  produce 16 kHz mono signed 16-bit little-endian PCM. Binary frames go to the
  existing `/api/in/audio/stream` WebSocket. A bounded queue prevents an outage
  from building an unbounded recording backlog.
- The existing `/api/out/audio` long poll provides chunked MP3. AudioFileStream
  and AudioQueue decode and play it incrementally. Unsupported output formats
  fail rather than silently producing no sound. The playback backlog is capped
  at 2 MiB. The current TTS configuration must emit MP3.
- Recognition partials from `/api/out/text` stop local playback for barge-in.
  Transcripts and chat history are not displayed on the car screen.
- During a native session the phone web stage is removed, preventing duplicate
  microphone capture and competing voice subscribers. The phone shows an End
  action instead. Audio background mode allows a session while the phone is
  locked; credentials already use AfterFirstUnlockThisDeviceOnly accessibility.
- Network failures, audio-route changes, and a stalled capture watchdog end the
  session. Retry opens a fresh authenticated session. There is no silent automatic
  reconnection or recording after an interruption.

Grant native microphone permission before driving using **Agents → CarPlay** on
iPhone. A missing permission on CarPlay directs the user to phone setup rather
than requesting permission on a locked phone.

## Validation before release

Build on a Mac with Xcode, then use an iOS 26.4+ CarPlay Simulator and an approved
physical-device provisioning profile. Simulator compilation with an older SDK
can check interfaces but cannot verify the new category's runtime support.

Required manual coverage:

1. Ordinary iPhone launch, pairing, share links, and iPad layout still work.
2. CarPlay-only cold launch loads the persisted roster; empty roster and denied
   microphone show phone setup guidance.
3. Choosing an agent displays voice control before recording; both car and phone
   End controls, system Back, app switching, and car disconnect stop all capture.
4. Speak and receive incremental MP3 output; speak over a response to interrupt
   it. Verify acoustic echo cancellation on actual car hardware.
5. Lock the iPhone after starting and cold-launch CarPlay with the phone locked
   after its first unlock. Confirm capture and output remain available.
6. Interrupt with a phone call, change an audio route, lose network, restart the
   core, and expire the session cookie. Confirm capture stops visibly and Retry
   establishes a fresh session without duplicate audio.
7. Check native templates in light/dark mode and on touch/rotary car controls.

### Checks performed on 2026-10-12

The app and embedded share extension build successfully with Xcode 26.2 for
iOS Simulator. A fresh iPhone 17 / iOS 26.3 simulator installs and launches the
app normally; the existing welcome screen and pairing entry points render.
The native audio check passes using real Apple audio APIs on
macOS: 16/44.1/48 kHz mono and stereo inputs produce 16 kHz mono signed Int16
with the expected sample count and amplitude; irregularly split MP3 chunks
decode and drain, and cancellation suppresses stale playback completion.

Run the audio check on a Mac (with ffmpeg installed) from `app/apple/ios`, using
a temporary directory for the fixture and executable:

```sh
test_dir=$(mktemp -d /tmp/hi-agent-audio.XXXXXX)
ffmpeg -v error -f lavfi -i anullsrc=r=24000:cl=mono -t 0.8 "$test_dir/fixture.mp3"
xcrun swiftc -parse-as-library -swift-version 5 \
  HiAgentIOS/CarPlayAudioPlayer.swift HiAgentIOS/CarPlayPCMEncoder.swift \
  tests/CarPlayAudioPlayerCheck.swift -o "$test_dir/audio-check"
"$test_dir/audio-check" "$test_dir/fixture.mp3"
```

The available simulator runtime is iOS 26.3, so these checks do **not** establish
CarPlay-category runtime behavior, locked-phone operation, live-core end-to-end
voice, or acoustic echo cancellation in a vehicle. Those remain release gates.

## References

- https://developer.apple.com/download/files/CarPlay-Developer-Guide.pdf
- https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.carplay-voice-based-conversation
