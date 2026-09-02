# Android Client

The Android client lives at `app/android` and targets phones and tablets from
one target. It is a client of a remote core and never hosts one.

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

It is a standalone Gradle build. It does not link the Rust core, `hi-app`, or
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

Four things do not port, and each is solved in one named place rather than
spread around.

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

Or open `app/android` in Android Studio.

## Not yet

Push notifications, physical-device coverage, and release signing and
distribution. There is no TestFlight equivalent; the intended first channel is a
signed APK hosted alongside the site rather than Play, which is unavailable in
the market this is aimed at. No signing config is committed, so `make android-apk`
produces an unsigned artifact today.
