# Hi Agent Windows

The native Windows app: a client of a core, and the host of the one on this
machine.

This target owns the Windows shell and client state:

- the process, the message loop, and the notification-area icon;
- **the engine** — `hi-agent.exe` runs as this app's supervised child;
- pairing and the local roster;
- long-lived credentials in Windows Credential Manager;
- short-lived `hi_surface` session cookies;
- health checks;
- the WebView2 that renders the core face.

It talks to a core over its HTTP API. It does not link the Rust core, `hi-wire`,
or an FFI layer, and shares no code with the Apple or Android clients.

See [`docs/platforms/windows.md`](../../docs/platforms/windows.md) for why this
is the first app written in the target shape, how the engine is supervised, and
the four places Windows differs from the phones.

## Open

Open `HiAgentWindows.sln` in Visual Studio 2022 with the *.NET Desktop* and
*Windows App SDK* workloads, or from the repository root on a Windows host:

```sh
make win-app
```

The .NET SDK 8 is the only other requirement. The app is unpackaged and
self-contained — both the .NET runtime and the Windows App SDK are published
with it — so the installer needs no prerequisite chain. WebView2's Evergreen
runtime is the exception, and ships with Windows.

**A Windows host is not optional here.** WinUI 3's XAML compiler and the Windows
App SDK run nowhere else; unlike `make exe`, there is no cross build to fall back
on.

## First slice

1. Starting, supervising and shutting down the local engine, with its data
   directory outside the install location.
2. Adopting an engine already answering on 12358 instead of starting a second.
3. Loading the local core's face with no session exchange, because loopback is
   ungated.
4. Adding a remote core by address and pairing code.
5. Keeping the credential in Windows Credential Manager.
6. Checking whether cores answer, and saying so when one stops.
7. Switching between cores from the tray.
8. Renewing rejected or expired web sessions without exposing the credential to
   JavaScript.
9. Granting camera and microphone capture only to the attached core's exact web
   origin.
10. A title bar that matches the face's own background in both appearances.

Notifications, start-at-login, Authenticode signing, and every capability
mechanism (screen, input, accessibility, hotkey) are follow-up work — the last
group because the seam they would answer on does not exist yet.

## Status

**Never compiled.** There is no Windows machine and no .NET SDK on any host this
repo is developed from. Everything here is written blind, to be fixed forward on
the first real build; `docs/platforms/windows.md` § *Verification* names the two
API details most likely to be wrong and why the package versions are wildcards
rather than pins.

## Dependencies worth knowing about

`H.NotifyIcon.WinUI` for the tray. Hand-rolling `Shell_NotifyIcon` means a
message-only window, a subclassed `WndProc`, and icon lifetime across Explorer
restarts — all of it the kind of code that looks right and is not, which is the
wrong thing to write with no way to run it.

Credential Manager directly, rather than a wrapper. It is the store the Keychain
and the Android Keystore stand in for on the other two clients, and reaching it
is four P/Invokes.
