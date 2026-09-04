# Android Client

The Android client lives at `app/android`. It is a client of a remote core and
never hosts one.

One Gradle build, two device shapes: this document is the `mobile` flavor —
phones and tablets — and [`android-tv.md`](android-tv.md) is the `tv` one. The
client layer under `core/` is shared and identical; only the shell differs.

## Ownership

The Android app owns:

- its roster and current-core selection;
- the long-lived credential in the Android Keystore;
- session exchange and health checks;
- `WebView` lifecycle;
- Android camera, microphone, notification, and background mechanisms as they
  are added.

The core remains the authority for identity, credential issuance and
revocation, memory, cognition, and channel behavior.

It is a standalone Gradle build. It does not link the Rust core or
`hi-wire`, and shares no client code with the Apple target — the two speak the
same documented API and are otherwise unrelated, which is the same independence
[`apple-ios.md`](apple-ios.md) describes.

## Connection

Identical to iOS, because the wire is the same. The app presents its credential
to `POST /api/session`, installs the returned short-lived cookie into the
`WebView`'s `CookieManager`, then loads the core address.

The credential never enters JavaScript, `SharedPreferences` in the clear, or the
WebView's storage. `POST /api/pair`'s `hiagent://pair` URL is registered as an
`intent-filter`, so a QR the core draws for any client scans here unchanged.

If the web session is rejected, the shell exchanges the Keystore credential
again and reloads with the new cookie. Foregrounding and network restoration
also re-check the selected core. Camera and microphone capture requests are
granted only when the WebView reports the selected core's exact scheme, host,
and port, and only for permissions the app itself holds.

## Where Android differs from iOS

Five things do not port, and each is solved in one named place rather than
spread around.

**Nobody raises the capture prompts.** WebKit asks for the microphone and camera
itself when a page calls `getUserMedia`, so the iOS client has no code at this
seam — and that absence is what hid the gap here for as long as it lasted.
Android hands `onPermissionRequest` to the app and expects the app to already
hold the runtime permission. `RECORD_AUDIO` was declared in the manifest and
never requested, so every audio request took the deny branch and the microphone
was dead while the camera worked, because the QR scanner asks for `CAMERA` on its
own account. `CoreWebView`'s chrome client now asks at the moment the face reaches
for the device, denies the pending request either way so a queued grant never
reads as a hung camera, and asks at most once per permission per view — Android
answers a twice-refused permission instantly and invisibly, and a prompt nobody
can see is a loop rather than a prompt.

**Cleartext to a LAN core.** iOS gets `NSAllowsLocalNetworking`, which exempts
local names and private literals from ATS while still refusing plain HTTP to a
public host. Android's network security config matches by domain name, and a LAN
core has an address rather than a name. So cleartext is permitted at the manifest
layer and narrowed in `CoreClient.normalizeBaseUrl`, which accepts `http://` only
for loopback, private and link-local literals, single-label hostnames, and
`.local` names. A public host over plain HTTP is refused before it can reach the
roster. Names are judged by shape and never resolved.

**Reading a navigation's status.** There is no
`decidePolicyFor navigationResponse`, so a main-frame 401 cannot be cancelled.
`onReceivedHttpError` reports the status instead, and the shell suppresses its
own Ready event for that navigation so a rendered "unauthorized" body is never
shown while the credential is being re-exchanged. A 401 met by the face's own
`fetch` long after load is caught by a document-start script and a one-method JS
bridge, as on iOS.

**Media capture needs a settings flag.** `mediaPlaybackRequiresUserGesture`
defaults to true and gates the page's `AudioContext` — the graph the microphone
runs through — so leaving it costs the mic and the agent's voice while the camera
appears to work. This is the same trap as the iOS
`mediaTypesRequiringUserActionForPlayback` default, described at length in
`CoreWebView.swift`.

**Safe-area insets.** The face reads `env(safe-area-inset-*)` and holds its own
content clear. WebView only fills those variables when the window is laid out
under the cutout, which needs `enableEdgeToEdge()` plus `shortEdges` in the
theme; letting the window also inset the WebView would apply the notch twice.

## Build

With the Android SDK installed and `ANDROID_HOME` set:

```sh
make android      # debug APK
make android-apk  # unsigned release APK
```

The television flavor has its own pair, `make android-tv` and `make android-tv-apk`.

Or open `app/android` in Android Studio.

## Not yet

Push notifications, physical-device coverage, and release signing and
distribution. There is no TestFlight equivalent; the intended first channel is a
signed APK hosted alongside the site rather than Play, which is unavailable in
the market this is aimed at. No signing config is committed, so `make android-apk`
produces an unsigned artifact today.
