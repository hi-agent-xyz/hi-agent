# Hi Agent Android

The native Android client for a remote hi-agent core, in two device shapes from
one Gradle build:

| Flavor | Shape | Shell | Doc |
|---|---|---|---|
| `mobile` | phones, tablets | touch, QR scanning, home-screen launcher | [`docs/platforms/android.md`](../../docs/platforms/android.md) |
| `tv` | televisions | D-pad, typed pairing, leanback launcher | [`docs/platforms/android-tv.md`](../../docs/platforms/android-tv.md) |

Everything under `core/` is shared and identical — the roster, the credential,
session exchange, the cleartext policy and its tests. Only the shell differs.

This target owns the Android shell and client state:

- pairing and the local roster;
- long-lived credentials in the Android Keystore;
- short-lived `hi_surface` session cookies;
- health checks;
- the authenticated `WebView` that renders the core face.

The client talks directly to a core over its HTTP API. It does not link the Rust
core, `hi-wire`, or an FFI layer, and shares no code with the Apple
client.

See [`docs/platforms/android.md`](../../docs/platforms/android.md) for the
connection model and the four places Android's platform differs from iOS.

## Open

Open `app/android` in Android Studio, or from the repository root:

```sh
make android      # the handset flavor
make android-tv   # the television flavor
```

`ANDROID_HOME` must point at an Android SDK; the Gradle build compiles against
platform 37 and will fetch it if the SDK does not have it yet. `make android`
also runs the JVM unit tests.

## First slice

Everything the iOS client's first slice covers:

1. Pairing a core with its base address and one-time pairing code.
2. Pairing from a `hiagent://pair` deep link or the QR code shown by the core.
3. Keeping the resulting credential in the Android Keystore.
4. Checking whether paired cores answer.
5. Switching between paired cores.
6. Loading the existing core web face with the exchanged session cookie.
7. Renewing rejected or expired web sessions without exposing the credential to
   JavaScript.
8. Retrying after foregrounding or network restoration, with native offline and
   connection error states.
9. Granting camera and microphone capture only to the paired core's exact web
   origin.

Push notifications, physical-device coverage, and release signing remain
separate follow-up work.

The app does not host a core on Android.

## Dependencies worth knowing about

QR scanning is CameraX frames decoded by ZXing, not ML Kit. ML Kit's barcode
scanner is delivered through Google Play services, which is exactly the
dependency that does not exist on many of the handsets this app is for. Nothing
in this target requires Play services.

The credential store is the Android Keystore directly rather than
`androidx.security:security-crypto`, which wraps the same primitives and adds a
dependency for about forty lines of code.
