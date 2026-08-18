# Native Client API

The native Apple and Android clients talk directly to a core. They do not link
the Rust core and do not share a client implementation.

## Pairing and sessions

`POST /api/session` is the only bootstrap endpoint.

Request:

```http
Authorization: Bearer <pairing-code-or-credential>
Content-Type: application/json
```

```json
{ "label": "Xiaoyuan's iPhone" }
```

Response:

```json
{ "id": "<surface-id>", "credential": "<new-credential-or-null>" }
```

The response also sets a short-lived `hi_surface` cookie. The client stores
`credential` in platform secure storage and passes the cookie to the core face.
When a credential is presented instead of a pairing code, `credential` is null
and the client keeps the credential it already has.

`label` identifies the authorized device at the core. A local roster name for
the core is app state and is not sent through this endpoint.

## Health

`GET /healthz` is open and returns `200` when the core process answers.

## Face

After session exchange, the client opens the core base URL in its native
WebView. The existing web face owns the channel protocol and sends
`X-HI-Surface: 1` on state-changing browser-shaped requests.

The long-lived credential must never be passed to the WebView.
