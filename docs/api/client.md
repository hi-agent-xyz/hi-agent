# Native Client API

The native Apple and Android clients talk directly to a core. They do not link
the Rust core and do not share a client implementation.

## Pairing and sessions

`POST /api/session` is where every path ends up: it is what turns a credential — or a
one-time code — into a session.

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

The response also sets an `hi_surface` cookie. The client stores `credential` in platform
secure storage and passes the cookie to the core face. When a credential is presented
instead of a pairing code, `credential` is null and the client keeps the credential it
already has.

The cookie lasts thirty days and survives a core restart; a use more than a day into its
life extends it, and the gate re-sends the same cookie with a fresh `Max-Age`. A native
client does not have to care — it re-exchanges its credential on every open — but it must
not assume a cookie it was handed has already lapsed.

`label` identifies the authorized device at the core. A local roster name for the core is
app state and is not sent through this endpoint.

## Asking to be let in

The second bootstrap path, for a device that has neither a credential nor a code: a TV, or a
phone in another room. Both halves are open — the caller is by definition unauthorized.

```http
POST /api/access/request
Content-Type: application/json

{ "label": "the living room TV" }
```

```json
{ "id": "<request-id>", "code": "418320", "secret": "<poll-secret>", "expires_in": 600 }
```

Show `code` to the person. The **same six digits** appear on the core's `reach` view beside
`label`, so they can tell which waiting device is the one in their hand. It is not a secret
and authorizes nothing.

Then poll, presenting `secret`:

```http
GET /api/access/request
Authorization: Bearer <poll-secret>
```

```json
{ "state": "pending" | "approved" | "denied" | "expired", "credential": "<or null>" }
```

`approved` carries the credential **exactly once** — store it and exchange it at
`POST /api/session` as usual. `denied` and `expired` are deliberately different answers:
"somebody said no" and "nobody answered" are different things to put on a screen. An unknown
secret is answered `expired`, so a client cannot tell a wrong secret from a lapsed request —
and does not need to, since both mean ask again.

Poll every couple of seconds. There is no long-poll and no push: the request's whole life is
ten minutes, and somebody is watching a code on a screen for most of it.

A `429` on the `POST` means the core already has its maximum of devices waiting (eight). A
`404` or `503` means nothing is answering at that address — for a client resolving a name, that
is "no agent answers to that name yet".

## Health

`GET /healthz` is open and returns `200` when the core process answers.

## Face

After session exchange, the client opens the core base URL in its native
WebView. The existing web face owns the channel protocol and sends
`X-HI-Surface: 1` on state-changing browser-shaped requests.

The long-lived credential must never be passed to the WebView.
