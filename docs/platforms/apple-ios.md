# Apple iOS Client

The first Apple client lives at `app/apple/ios` and targets iPhone and iPad from
one native iOS target.

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

## Build

On macOS with Xcode installed:

```sh
make ios
```

For a physical device, open `app/apple/ios/HiAgentIOS.xcodeproj`, select a
development team, and run the `HiAgentIOS` scheme.
