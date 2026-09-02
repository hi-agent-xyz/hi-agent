# Hi Agent iOS

The native iPhone and iPad client for a remote hi-agent core.

This target owns the iOS shell and client state:

- pairing and the local roster;
- long-lived credentials in the Keychain;
- short-lived `hi_surface` session cookies;
- health checks;
- the authenticated `WKWebView` that renders the core face.

The client talks directly to a core over its HTTP API. It does not link the
Rust core, `hi-app`, `hi-wire`, or an FFI layer.

## Open

Open `HiAgentIOS.xcodeproj` on a Mac with Xcode. The target supports both iPhone
and iPad through the same iOS target.

Set a development team and bundle identifier in Xcode before installing on a
device. The default bundle identifier is `com.xiaoyuanzhu.hiagent.ios`.

## TestFlight

`.github/workflows/ios-testflight.yml` archives and uploads the app whenever
`main` changes the iOS app or `VERSION`. It can also be started manually. Each
run uses `<workflow run>.<attempt>` as the App Store build number, so retries
remain uploadable.

The job expects an Apple silicon self-hosted GitHub Actions runner with the
labels `self-hosted`, `macOS`, `ARM64`, and `macmini`. Install the current
Xcode, select it with `xcode-select`, accept its license, run Xcode's first
launch setup, and keep the runner updated for the action versions in the
workflow.

Create the bundle ID and app record in App Store Connect, then configure:

- Repository variable `APPLE_TEAM_ID`: the 10-character Developer team ID.
- Optional repository variable `IOS_BUNDLE_ID`: defaults to
  `com.xiaoyuanzhu.hiagent.ios`.
- Secret `APP_STORE_CONNECT_ISSUER_ID`: the API key issuer UUID.
- Secret `APP_STORE_CONNECT_KEY_ID`: the API key ID.
- Secret `APP_STORE_CONNECT_API_KEY_P8_BASE64`: the downloaded `.p8` file,
  base64 encoded as one line.

The API key must be allowed to upload the app and use cloud-managed
distribution certificates. An Admin team key is the direct setup; a less
privileged identity needs explicit cloud-managed certificate access.

On macOS, encode the downloaded key with:

```sh
base64 < AuthKey_KEYID.p8 | tr -d '\n'
```

Xcode performs automatic provisioning and cloud signing during the job. No
distribution certificate or provisioning profile is stored on the Mac mini.
After Apple processes the first upload, configure TestFlight groups and export
compliance in App Store Connect. External testers also require Apple's beta app
review before they can install the build.

## First slice

The app currently supports:

1. Pairing a core with its base address and one-time pairing code.
2. Pairing from a `hiagent://pair` deep link or the QR code shown by the core.
3. Keeping the resulting credential in the iOS Keychain.
4. Checking whether paired cores answer.
5. Switching between paired cores.
6. Loading the existing core web face with the exchanged session cookie.
7. Renewing rejected or expired web sessions without exposing the credential to
   JavaScript.
8. Retrying after foregrounding or network restoration, with native offline and
   connection error states.
9. Granting camera and microphone capture only to the paired core's exact web
   origin.
10. Showing the agent your screen from a button — the phone's half of the desktop's
    ⌘⌘ "come and see this".

Push notifications, physical-device coverage, and release packaging remain
separate follow-up work.

## Showing your screen

`ShowScreenIntent` ("Show My Screen") is an App Intent that takes an image, hands it
to the attached core on the `file` channel with a note saying it is the person's
screen, and opens the app. The person wires it to a button themselves:

    Take Screenshot  →  Show My Screen        (a shortcut)
    Settings → Action Button → Shortcut       (or Back Tap, or Control Centre)

**The intent takes the picture as a parameter rather than capturing one, because no
iOS app can photograph another app's screen.** The Shortcuts *Take Screenshot* action
can, and the Action Button runs a shortcut over whatever app is in front without
leaving it — so the OS holds the screen and this app is only the carrier. The order
matters for the same reason: the screenshot has to exist before `openAppWhenRun`
brings Hi Agent forward, or the picture is of Hi Agent.

`ShowScreenSetupView` (Cores sheet → *Show your screen with a button*) is the
in-app instructions. A signed `.shortcut` file served from the website would collapse
its first three steps into one import tap; that needs a Mac to sign it and somewhere
to host it, and is not built.

The app does not host a core on iOS.
