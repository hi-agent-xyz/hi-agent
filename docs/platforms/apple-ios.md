# Apple iOS Client

The first Apple client lives at `app/apple/ios` and targets iPhone and iPad from
one native iOS target.

## iPhone and iPad are one app

There is no iPad target, no iPad code path, and no iPad build. `TARGETED_DEVICE_FAMILY`
is `1,2`, the TestFlight job archives `generic/platform=iOS`, and the same binary
installs on both. A tablet is a phone client with more room, and every difference
below follows from the room rather than from the hardware.

**The room decides the layout, not the device.** `Theme.measure` is the widest the
shell's own content is allowed to get, and `hiMeasure()` centres a screen's column
inside whatever it is given; `EnvironmentValues.isRoomy` is the regular-width test
for the two places where the *arrangement* changes and not just the width:

- the Welcome screen's actions travel with the column instead of pinning to the
  bottom edge — that edge is where a thumb already is on a phone and merely the
  furthest point on a tablet;
- the stage chrome groups its two capsules at the leading edge instead of pushing
  them to opposite corners, which at 13 inches stops reading as one control.

Both are read off the size class, so an iPad in a narrow Split View column gets the
phone's arrangement and the same iPad at full width does not. The measure is wider
than every iPhone, so `hiMeasure()` is inert there and the phone layout is untouched.

**The face needed nothing.** `src/appearance/web/src/lib/shape.ts` already answers
`phone` only for `narrow AND coarse`, so a tablet-width webview gets the same `wide`
shape as a desktop window, and a Split View column narrow enough to be phone-shaped
gets the pushed-page stack and its back-swipe. That query is live, so rotating or
resizing re-answers it.

**Where the gesture lives differs, and that one is about the hardware.**
`ShowScreenPlacement` picks the setup instructions off `userInterfaceIdiom`, because
an iPad has no Action Button and no Back Tap however wide its window is — and the
iPhone copy sent an iPad owner to a Settings row that does not exist.

### Decisions

- **No multiple windows.** `UIApplicationSupportsMultipleScenes` stays unset. Not
  because two windows would conflict — [stage.md](../arch/stage.md) puts the cursor
  on the stage and has *every attached window render it*, so a second window is well
  defined — but because it would then show the same stage as the first, which is not
  worth a scene lifecycle. Split View and Slide Over, where the second app is
  somebody else's, work and are the multitasking that pays.
- **Still remote-only.** An iPad hosts no core, exactly as an iPhone hosts none. The
  tablet changes how much room the client has, not what the client is.

### Verified, and not

The layout above was watched on an iPad Pro 11-inch simulator against a live core
(2026-09-03): pairing, the stage chrome grouped, the face in its `wide` shape, and an
iPhone 17 run alongside showing the phone layout unchanged. **No build of this has
ever run on physical iPad hardware**, and `ShowScreenPlacement`'s iPad copy — the
Control Centre and Home Screen routes — has never been followed through on a device.

## Ownership

The iOS app owns:

- its roster and current-core selection;
- the long-lived credential in Keychain;
- session exchange and health checks;
- `WKWebView` lifecycle;
- iOS camera, microphone, notification, and background mechanisms as they are
  added.

The core remains the authority for identity, credential issuance and
revocation, memory, cognition, and channel behavior.

## Connection

The app talks directly to the selected core. It presents its credential to
`POST /api/session`, installs the returned short-lived cookie into the
`WKWebView` cookie store, then loads the core address.

The credential never enters JavaScript, `UserDefaults`, a plist, or the
WebView's storage.

The core's `POST /api/pair` response also includes a `hiagent://pair` URL with
the core address and one-time code. The Reach view encodes that URL into its QR
code. iOS accepts the URL from the Camera app or scans it in-app, then shows the
native pairing form before exchanging the code.

If the web session is rejected, the shell exchanges the Keychain credential
again and reloads with the new cookie. Foregrounding and network restoration
also re-check the selected core. Camera and microphone capture requests are
granted only when WebKit reports the selected core's exact scheme, host, and
port.

## Build

On macOS with Xcode installed:

```sh
make ios
```

For a physical device, open `app/apple/ios/HiAgentIOS.xcodeproj`, select a
development team, and run the `HiAgentIOS` scheme.
