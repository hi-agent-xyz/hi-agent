# Keys a person typed

## What this is for

Somebody pastes an API key into the conversation to get something done — "here's the key,
check the endpoint works". They were thinking about the endpoint, not about the key. That
unconscious moment is the whole subject of this document.

The key is written to an ordinary file in the drive, and the text that enters a model
session names the file instead of carrying the value. The agent can still spend the
credential; it just does so by path.

## What this is not

**Not a vault.** The secret is transparent to the host, to the drive, to the person, and
to the agent the moment the agent decides to go and read the file. That decision is
allowed and unguarded. Nothing here withholds anything from a session that asks.

This is the difference the design turns on: **an accident is prevented, a decision is
not.** A person typing a key mid-sentence did not choose to send it to a model. An agent
running `cat` on a secret file did. Only the first is this boundary's business.

No prompt may describe this as a safe place to put credentials, offer to hold them, or
invite more. `reaction.md` carries that rule, because Reaction is the rung in the room.

## The two seams

Exactly two, and nothing else in the codebase participates.

| | Where | What it does |
|---|---|---|
| **Detect** | `SensitiveDataFilter::file_secrets`, called from `POST /api/in/text` | Scans one inbound human message. Each credential found is written to `drive/accounts/secrets/<name>.txt`. The message is not modified. |
| **Substitute** | `AgentSession::prompt` | Replaces every known stored value with `⟨secret: <path>⟩`, by exact match. No detector runs here. |

Detection runs once, on arrival. Substitution runs on every turn — which is the point:
the journal snapshot renders the same message back into a prompt on the twentieth turn as
readily as the first, and a filter that only saw the live signal would have leaked it on
turn two. Masking is a property of the text, not of the moment it arrived.

`AgentSession::prompt` is the only door into a model session, so every rung is covered by
construction — the live message, the snapshot, a worker brief quoting the person, and
whatever gets added next.

## What is deliberately untouched

- **The journal and the conversation.** `/api/out/text`, `GET /api/messages` and
  `memory/raw/` all carry exactly what was typed. The person is not the one being kept
  from their own key.
- **Tool results.** A host reader hands back the bytes on disk.
- **The system prompt**, and agent-to-agent mail.
- **Codex's own shell.** A command the agent runs returns its output straight into the
  model's context without passing through hi-agent at all.

The last one means the substitution is a **convention, not a rail**: a session that runs
`cat drive/accounts/secrets/x.txt` gets the value, and no code stops it. Enforcing
otherwise would mean denying the secrets directory to the sandbox and making
`hi_http_request` the only consumer, which would break using an arbitrary CLI against a
real API — the capability this design exists to preserve.

## Secrets only

PII is not detected and not masked. Masking an address or a phone number costs the agent
the ability to do the thing it was asked to do, and unlike a credential there is no file
to hand it instead. `redact-core` supplies the detectors; only its bearer-credential
entity types are enabled.

`redact-core` 0.10.0 slices a ±50-**byte** context window around each hit without checking
char boundaries, so detection reads a byte-for-byte ASCII stand-in of the message (each
non-ASCII char's bytes replaced by newlines). Offsets still address the original, the
panic cannot fire, and a key embedded in Chinese text is found at all — `regex`'s `\b` is
Unicode-aware, so `是` is a word character and `我的key是sk-proj-…` previously matched
nothing.

## Secret files

There is no store, database, schema, or metadata record. Each retained credential is one
ordinary text file:

```text
drive/accounts/secrets/openai-api-key.txt
```

The path is the stable reference. The complete file content is the exact credential, with
no fields. Files are written atomically and owner-only on Unix, and are plaintext at rest.
Copying `drive/` copies both the credential and every reference to it.

Every stored value is held in memory and reloaded on write, because substitution runs on
every prompt and may not touch the disk.

## Spending one

### HTTP

`hi_http_request` takes the path as `auth_ref`. The host reads the file, injects the
value, follows no redirects, and returns the response as it came back.

### CLI

The command carries the path and reads the value at execution time:

```sh
OPENAI_API_KEY="$(cat drive/accounts/secrets/openai-api-key.txt)" some-cli
```

Prefer forms that keep the value out of `argv` and out of anything printed back.

## Retention

Detected secrets are retained automatically. The *this one* / *all* / *none* preference in
[`data.md`](data.md#keys-passwords-and-the-one-question) is not implemented, and the
prompts say so rather than implying a choice was offered.

## Limitations

1. Secret files are plaintext at rest.
2. Codex's own shell output never crosses a seam (see above). The guarantee is against
   accident, not against a session that goes looking.
3. Only `POST /api/in/text` is scanned. A key arriving by voice or inside an uploaded file
   is not detected — STT will not reproduce one accurately, and a file the agent opens is
   the agent deciding to look.
4. A message over 1 MiB is delivered unscanned rather than refused, and says so in the log.
