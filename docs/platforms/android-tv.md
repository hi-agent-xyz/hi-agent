# Android TV Client

The television client is the `tv` flavor of [`app/android`](../../app/android), not a build
of its own. It is a client of a remote core and never hosts one.

## Why a flavor and not a second target

`app/apple/ios` and `app/android` share no code deliberately: different languages, different
wire clients, and the only thing between them is the documented API in
[`../api/client.md`](../api/client.md). A television is not that kind of distance. It is the
same platform and the same language, and everything under `core/` — the roster, the Keystore
credential, session exchange, and `normalizeBaseUrl`'s cleartext policy with the eight tests
that are the only tests in this target — is already correct for it. A second copy of a
security policy is a copy that drifts.

So the split is the shell and only the shell:

    src/main     core/, AppModel, CoreWebView, the theme      both
    src/mobile   the handset's screens, the QR scanner        handset
    src/tv       the television's screens                     television

CameraX and ZXing are `mobileImplementation`, so the scanner is not merely unreachable on a
television but absent from its APK.

One `applicationId` for both flavors, unsuffixed: no device is both shapes, so the two
installs never meet. Publishing both to Play under one listing would need distinct version
codes; that is a distribution question, and Play is not the intended channel.

## What a remote changes

**Pairing is typed.** The QR is the path a core offers and a television has no camera, so
the handset's fallback is the whole screen here. An address and a one-time code, once per
core, after which the roster remembers the core and the Keystore remembers the credential.
Nothing about the wire changes for it: `POST /api/session` takes the same code from a
television, and the core is not told what kind of device spent it. The `hiagent://pair`
intent filter is deliberately **not** declared here — nothing on a television can deliver
that link, and an entry point on paper only is worse than none.

**Back is the navigation, and it is a ladder out of the app.** The face's own surfaces close
first, then the shell's chrome appears, then the system closes the activity. The face's rungs
are reached through the bridge described in [`../arch/stage.md`](../arch/stage.md) §
*The television is the room* — the face reports a depth, the shell dispatches `hi:back`,
because Android never hands `KEYCODE_BACK` to a WebView and the shell cannot ask a page a
synchronous question. The chrome Back summons does not time out: a control that vanishes
mid-decision makes leaving the app a matter of reflexes.

**The picture has an edge nobody reports.** Televisions overscan — the panel crops the frame
it is sent, by an amount that is the set's business — and there is no inset to read, because
`env(safe-area-inset-*)` is zero on a set cropping forty pixels just as it is on one cropping
none. The platform's fixed 5% stands in for the measurement it cannot make: 48dp and 27dp on
the 960x540dp frame Android TV lays 1920x1080 out as, held by `Tv.overscan` in the shell and
by the `--hi-safe-*` tokens in the face.

**The screen is never held awake.** A face left up in an empty room is a still image on a
panel that may be OLED, so the system's own screensaver is left to do its job. Waking a room's
television to say something is a different capability, and it does not exist on any client.

**No `enableEdgeToEdge`.** The inset that call exists to expose is a display cutout, and a
television has none.

## The face on a television

The web face is the same build, told which host it is on: the shell loads the core's address
with `?shape=tv`, the way the macOS window asks for `?chrome=titlebar`. That is what arms
arrow-key focus movement, the overscan margin, and an unconditional focus ring. The design
is in [`../arch/stage.md`](../arch/stage.md); the reason it is declared rather than detected
is there too, and it is the load-bearing decision of this client.

## Build

With the Android SDK installed and `ANDROID_HOME` set:

```sh
make android-tv      # debug APK, plus the shared unit tests
make android-tv-apk  # unsigned release APK
```

## Not watched

**Nothing in this client has run on a television or an emulator.** The build host's SDK
carries no Android TV system image, and installing one needs the user's go-ahead. Every
behaviour below is therefore designed and compiled, not observed, and each is a specific
thing to check on the first real screen rather than a general worry:

- whether the WebView takes focus without `focusOnAttach`, and whether the arrows reach the
  page at all;
- whether a Compose `TextField` on the leanback IME lets up and down leave it again, or
  keeps them for a caret;
- whether the focus ring is legible from a sofa, and whether the `wide` type scale is;
- whether the remote's microphone reaches `getUserMedia`, which decides whether talking to
  the agent from a sofa works at all;
- whether the banner renders as a layer-list at the size the launcher asks for.

The handset flavor is in the same position and has been since it shipped —
see [`android.md`](android.md).

## Not built

Push notifications, release signing, and distribution, exactly as on the handset. A
television is also the one client where an agent that wants attention has no way to get it,
since it cannot wake the screen.
