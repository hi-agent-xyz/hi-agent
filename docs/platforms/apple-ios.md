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

## Sharing into the agent

Hi Agent is in the iOS share sheet for everything a share sheet can offer — a photo,
a file, a link, a selection. The target is `app/apple/ios/HiAgentShare`, a share
extension with **no interface**: every other share extension asks you to pick an
album or a recipient, this one has exactly one destination and nothing to ask.

Two doors, chosen by what was shared. Files go to `POST /api/in/file`; a link or a
selection goes to `POST /api/in/text`, because **a link is something a person says,
not an artifact they hand over**. Order matters in `Attachment.read`:
`public.file-url` conforms to `public.url`, so a document from Files answers yes to
both, and asking about links first would file every attachment as a path that stops
existing when the extension does.

**Nothing is written as a note.** What was shared is the whole of what was
communicated. The screen gesture still writes one — it has no conversation to type
into — and that asymmetry is the whole difference between the two carriers now.

### The extension queues; the app sends

The extension copies what was shared into an App Group container and stops.
`AppModel.deliverQueued()` does the HTTP, when the app next comes forward.

That is not the obvious arrangement — a share extension could do its own HTTP — and
four things fall out of it:

- **The extension never touches a credential**, so the Keychain does not have to be
  shared between the targets. `group.com.xiaoyuanzhu.hiagent` is the only entitlement
  either target gains; there is no keychain access group and no roster migration.
- **Nothing races the extension's lifetime.** A share extension is killed when its
  sheet goes away, which for a phone video over cellular is long before an upload
  finishes.
- **The queue is the retry buffer**, and it outlives both processes — a send that
  failed is still on disk after a crash or a week in a tunnel. The screen gesture was
  moved onto the same queue for this: its pending bytes used to die with the process.
- **The person ends up in the conversation**, which is the point of sharing something
  to your agent.

Each file is stored **already framed as a complete multipart body**. The extension has
to copy the bytes anyway — an item provider's URL is security-scoped and does not
survive the process — so it copies them into the shape the wire wants: one write, one
read, no second full-size copy on a phone that may be nearly full, and
`URLSession.upload(fromFile:)` streams it without ever holding a 3 GB video. One
request per file, so a drop of nine photos retries the one that failed.

### Opening the app: one call works, three do not

The extension's last act is to bring Hi Agent forward, because the reason to share
something to your agent is to say a thing about it. Getting there is the single most
misleading corner of this whole feature — **three of the four ways look right,
compile, and fail silently or crash on a current device**:

| | what happens |
|---|---|
| `UIApplication.shared.open` | does not compile under app-extension API rules |
| `NSExtensionContext.open(_:)` | documented Today-widget-only; completion fires `success = false` |
| responder-chain `openURL:` | force-returns NO since iOS 18, logging `BUG IN CLIENT OF UIKIT` |
| responder-chain `openURL:options:completionHandler:` | **crashes** inside UIKit's KVC probe |

What works is SwiftUI's own `OpenURLAction`, reached by instantiating
`EnvironmentValues()` directly rather than through a view's environment — public API,
no runtime reflection, no deprecated selector. This repo shipped the third row first
and it never once ran; the list, and the fix, come from a build of the same hand-off
that does work ([xiaoyuanzhu-com/my-life-db-apple](https://github.com/xiaoyuanzhu-com/my-life-db-apple)).

Two consequences shape everything around the call:

**It must be a Universal Link, so the extension needs a screen.** A custom scheme
goes through the machinery Apple has been closing; an `https` URL claimed in
`apple-app-site-association` is routed by Associated Domains, which is a different
path and the one still open. The link is `https://hi-agent.xyz/ios-share/<id>` — the
apex, not the person's own core, because a Universal Link only works for a domain in
the app's entitlement and `ana.hi-agent.xyz` is a tunnel into somebody's machine. The
claim is narrowed server-side to `/ios-share/*` so that a **shared view link never
opens somebody else's app**; see `backend/internal/server/applinks.go` in the
hi-agent.xyz repo. And because the open is reliably driven by a tap, the extension
has a screen with one button (`ShareView`) instead of the no-interface design it
started as.

**The teardown races the open.** Completing the extension request tears its UI down
and cancels an open that has not finished registering — silently, with nothing in any
log. So the open gets a runloop tick (50 ms, borrowed from the working build) before
`completeRequest`.

Still nothing depends on it: the drop is on disk before the button exists, and the
app drains the queue whenever it next comes forward. If the open fails the person
taps Hi Agent themselves and their share goes out — which is also the fallback for
anyone who picks *Not now*.

On Android none of this exists — `ACTION_SEND` starts the activity, so there is no
queue, no entitlement, and no hand-off. See [android.md](android.md).

### Verified, and not

`xcodebuild` builds the app with the extension embedded and validated
(2026-09-12). `NSExtensionActivationRule` is an enumerated dictionary rather than
`TRUEPREDICATE`: the latter builds fine and is an automatic App Store rejection, which
the embedded-binary validator says in as many words.

**Nothing here has been run.** Not on a device, not in the Simulator — no share sheet
has been opened, no drop has been queued, no queued drop has reached a core, and the
hand-off link has never been seen to open anything. Two things make this path
**unverifiable anywhere but a real device**: the App Group does not work in a
`CODE_SIGNING_ALLOWED=NO` build (ad-hoc sign it, as above), and a Universal Link
needs the association file fetched from the live domain against the real App ID.

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
`POST /api/session`, installs the returned cookie into the `WKWebView` cookie
store, then loads the core address.

The credential never enters JavaScript, `UserDefaults`, a plist, or the
WebView's storage.

### Adding an agent

**The user-facing noun is "agent".** Every screen says it; the wire does not — `/api/pair`,
`hiagent://pair`, `hi_surface` and `surface_credential` are shared with four other clients
and a rename there would buy nothing. iOS-local type names moved with the copy, so the app
stays coherent with itself: `AddAgentView`, `AddAgentRequest`, `AgentQRScannerView`.

Three ways in, in the order the sheet offers them:

1. **By name** — the hero. A single field rendered inline as `[ your agent ].hi-agent.xyz`,
   `POST /api/access/request`, then a six-digit code and a wait while the person approves it
   on the agent. This leads because it is the only one that works from another room: a scan
   only helps when you are standing at the machine showing the code, and if you are standing
   there you are standing where the Approve button is.
2. **A QR code** — `hiagent://pair`, carrying address and one-time code, from the Camera app
   or scanned in-app.
3. **A full address and a code**, folded into a disclosure, for a self-hosted agent with no
   name in the default zone. The stage's "Add again" opens straight onto it, because it
   arrives already knowing the address.

### Verified, and not

`make ios` builds, and all three states were watched in an iPhone 17 Simulator against a live
core on the same machine (2026-09-11, [journey 39](../user-journeys/39-add-a-remote-agent.md)):
the name field assembling `iloahz.hi-agent.xyz`, the waiting screen polling the real core and
showing the code that core had issued, and — after the request was approved from the other
side — the sheet dismissing and the stage opening the core's face. There is no tap driver over
SSH, so the waiting state was entered by seeding the invitation into a scratch build rather
than by typing a name.

**Two things are still unwatched.** The phone-to-desktop approval **on real hardware, across a
relay** — everything above was a Simulator talking to loopback. And a build from `make ios`
cannot complete the last step at all: it passes `CODE_SIGNING_ALLOWED=NO`, so the app has no
entitlements and `KeychainStore.save` fails `-34018`. That is not specific to this flow — the
pairing path stores its credential the same way — but it means **anything verified in the
Simulator past "exchange a credential" has to be built ad-hoc signed**:

```sh
xcodebuild -project app/apple/ios/HiAgentIOS.xcodeproj -scheme HiAgentIOS \
  -sdk iphonesimulator -configuration Debug \
  CODE_SIGN_IDENTITY="-" CODE_SIGNING_REQUIRED=YES CODE_SIGNING_ALLOWED=YES build
```

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
