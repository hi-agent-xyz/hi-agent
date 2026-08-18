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

## First slice

The app currently supports:

1. Pairing a core with its base address and one-time pairing code.
2. Keeping the resulting credential in the iOS Keychain.
3. Checking whether paired cores answer.
4. Switching between paired cores.
5. Loading the existing core web face with the exchanged session cookie.

The app does not host a core on iOS.
