# hi-agent

A reference implementation of the [human-interface](../human-interface/docs/human-interface.md) spec — a small Rust agent that talks over HTTP channels, delegates cognition to a `codex app-server` runtime (installed on first run) over JSON-RPC, and persists everything to JSONL.

## Status

design v0.1 · 2026-05-28 · v0 implementation complete · not load-tested.

## Quickstart

### Prerequisites

- Rust toolchain (2024 edition — `rustc` 1.85 or newer)
- hi-agent prefers the runtime your system already offers: if `node` and the
  `codex` CLI are both on your `PATH`, it uses them directly and downloads
  nothing. Installing those tools globally is also how you point hi-agent at your
  own runtime (e.g. to develop offline). Node is only esbuild's host — the agent
  itself is `codex app-server`, a native binary.
- If the system doesn't offer the full set, hi-agent falls back to a
  self-contained install: on first run it downloads the pinned Node and `npm ci`s
  the pinned `codex` CLI into an OS cache dir, then reuses that install on
  every subsequent start. That first run needs network access and the system
  `tar`; later runs are offline. macOS and Linux on x86_64/aarch64 are supported
  for auto-install.

### Build and run

Everything goes through the [`Makefile`](Makefile) — it is the supported way to
build, run, test, and package this repo. `make help` lists every target.

```sh
make build   # build the SPA, then the release binary
make run     # start the agent (creates ./data on first run)
make dev     # rust + vite dev servers, for working on the app
make test    # rust + web tests
```

`make build` runs `npm ci && npm run build` in `src/appearance/web` (so
rust-embed has a `dist/` to embed) and then `cargo build --release`, after
checking that every committed version stamp agrees with [`VERSION`](VERSION).
Running those commands by hand skips that ordering and those checks, so prefer
the target — a bare `cargo build --release` will happily produce a binary with a
stale or empty SPA.

### Versions and release artifacts

[`VERSION`](VERSION) is the project version source of truth. To update every
committed version stamp without committing or tagging:

```sh
make bump-version V=0.2.0
```

To cut a release from an up-to-date branch, run `make version`. It proposes the
next patch version, then updates the version files, commits `release v<version>`,
tags it, and pushes the commit and tag to `origin`.

Published desktop artifacts use the same platform-explicit convention as
Abacad:

```text
hi-agent-<version>-macos-apple-silicon.dmg
hi-agent-<version>-windows-x64.exe
```

### Verify it's alive

```sh
curl -X POST http://127.0.0.1:12358/api/in/text \
  --data-binary 'hello'
```

You should see `202 Accepted` and a fresh journal entry. To observe the shared text appearance, open its state stream:

```sh
curl -N http://127.0.0.1:12358/api/out/text
```

## Curl recipes

The most useful four:

```sh
# Current text state immediately, then whole-state replacements (Ctrl-C to close)
curl -N http://127.0.0.1:12358/api/out/text

# Send text
curl -X POST \
  --data-binary 'hey, are you there?' \
  http://127.0.0.1:12358/api/in/text

# Schedule a reminder (the router decides whether to call set_intent)
curl -X POST \
  --data-binary 'remind me at 21:00 to call mom' \
  http://127.0.0.1:12358/api/in/text

# Approve a pending action (id comes from the /approval long-poll JSON)
curl -X POST
  -H 'Content-Type: application/json' \
  -d '{"id":"<approval-uuid>","allow":true}' \
  http://127.0.0.1:12358/approval
```

### Text transcript contract

The conversation is an append-only message list. `GET /api/out/text` is one long-lived
NDJSON stream: the first line is the current window whole, and later lines append to it.

```json
{"reset":{"messages":[{"id":"0199…","ts":"2026-08-11T09:31:04Z","role":"user","text":"What day is it?"}],"interim":null}}
{"append":{"id":"0199…","ts":"2026-08-11T09:31:06Z","role":"agent","text":"Sunday."}}
```

Three things become messages and nothing else does: what the person typed or said, a
file they handed over, and one `hi_say` call — whole, never streamed in as it is generated.
Views, worker reports, clock wakes and tool calls are not conversation and stay out.

The backend owns the list. There are no client IDs, cursors, acknowledgements or read
receipts, and no window ever tells the backend what it has seen. The list is seeded from
the journal at boot, so a restart shows the conversation that was already happening;
`GET /api/messages?before=<id>` reads further back. The complete decision is in
[`docs/arch/text-transcript.md`](docs/arch/text-transcript.md).

## Architecture

One Rust process per agent. Inside it: an axum HTTP server, one reaction loop and a worker registry, a memory facade backed by channel logs, an in-process MCP hub that sessions reach over HTTP, and a heartbeat that injects synthetic signals when intents come due. Cognition is delegated: on first run hi-agent installs its runtime (downloading the pinned Node and `npm ci`-ing the pinned `codex` CLI into an OS cache dir), then on every start opens the reaction and standing-rung sessions plus any long-lived workers. Each session is one `codex app-server` subprocess spoken to over stdio JSON-RPC; the upstream credential rides a single env var that the thread's `model_providers` entry names via `env_key`, so the key never appears in the thread config — and so never in the wire frames the tap records.

```
  attached windows         hi-agent  (Rust process)              codex app-server subproc.
 ────────────────   ──────────────────────────             ──────────────────────────

  window A ──POST /api/in/text──┐
                         │   ┌─────────────────┐  JSON-RPC ┌────────────────────┐
  window B ──POST /api/in/vision▶├──▶│   axum server   │ ◀──stdio▶ │ session: reaction │
                         │   └────────┬────────┘           │  (ephemeral)       │
  windows ◀──GET /api/out/text──┘     │                    ├────────────────────┤
                                      ▼                    │ session: worker A  │
                             ┌─────────────────┐           │  (long-lived task) │
                             │     Reaction     │           ├────────────────────┤
                             │ shared input/output   │           │ session: worker B  │
                             │  worker reg.    │           │  (long-lived task) │
                             └────────┬────────┘           ├────────────────────┤
                                      │                    │ session: ...       │
                                      ▼          MCP       │                    │
                             ┌─────────────────┐ ◀──stdio▶ │  ← tool calls      │
                             │ in-proc MCP     │           └────────────────────┘
                             │ Memory journal  │ ← continuity
                             │ Heartbeat       │ ← aliveness
                             └─────────────────┘
```

See [`docs/impl.md`](docs/impl.md) for the full architecture document.

## Spec compliance (v0)

| Spec requirement | Status | Notes |
|---|---|---|
| `GET /` homepage | Y | Embedded Vite SPA, OG meta injected at request time |
| `POST /api/in/text` | Y | Body bytes are the signal; optional `X-HI-Stream` names its source |
| `GET /api/out/text` state stream | Y | Immediate whole current text state, then replacements; no identities, cursor, consumption, or catch-up |
| `GET /approval` long-poll | Y | JSON event; 5-minute timeout on the requesting side |
| `POST /vision` | 501 | Per v0 scope; body describes the omission |
| `POST /audio`, `GET /audio` | Y when configured | STT transcribes the body and routes the text; the router may reply via `speak(channel="audio")` which is synthesized back through TTS and broadcast on the long-poll. 501 on POST when `STT_PROVIDER` is unset. |
| `POST /touch`, `POST /smell`, `POST /taste` | 501 | Per v0 scope |
| One conversation | Y | All attached windows and channels share the same agent appearance and stream |
| Workers (parallel agent sessions) | Y | `hi_create_worker` MCP tool; one process-wide session per worker |
| Memory: `journal.jsonl` + `intents.jsonl` | Y | Append-only journal; intents file rewritten atomically on add/remove |
| Heartbeat (1 Hz, absolute intents) | Y | Synthetic `signal_in` on `channel: intent`, injected via the reaction |
| `Authorization: Bearer ...` | accepted/logged | Parsed and logged; not validated in v0 |
| Cron / relative intents | deferred | Per `docs/impl.md` Scope |
| Forgetting curve / significance / compaction | deferred | Per `docs/impl.md` Scope |
| Federation / e2e encryption / handle discovery | deferred | Per `docs/impl.md` Scope |

## Configuration

Env vars consulted at startup:

| Variable | Default | Purpose |
|---|---|---|
| `AI_API_KEY` | — | Upstream LLM credential. Optional — set here or via Settings (BYOK); without either the agent boots unconfigured. |
| `AI_API_BASE` | `https://api.openai.com/v1` | Upstream provider base; codex appends `/responses` to it. |
| `HI_AGENT_MODEL` | codex default | Model named on each `thread/start` |
| `HI_AGENT_EFFORT` | unset | Adapter `effortLevel` (e.g. `low` / `medium` / `high`) |
| `HI_AGENT_PERMISSION_MODE` | unset | Adapter `permissions.defaultMode` (e.g. `acceptEdits`) |
| `HI_AGENT_RUNTIME_DIR` | OS cache dir | Override the dir the runtime is installed into |
| `HI_AGENT_MCP_SOCK` | `<data_dir>/mcp.sock` | Unix socket the MCP hub listens on |
| `HI_AGENT_SHIM_BIN` | `current_exe()` | Program to re-exec as the MCP stdio↔socket shim |
| `RUST_LOG` | `info` | Standard `tracing-subscriber` env filter |

Managed cognition parameters (model, effort, permission mode) come from the
`HI_AGENT_MODEL` / `HI_AGENT_EFFORT` / `HI_AGENT_PERMISSION_MODE` env vars,
alongside the upstream credential and base URL (`AI_API_KEY` / `AI_API_BASE`).
In dev these load from `.env`; see [`.env.example`](.env.example). To use your
own runtime (or to skip
the first-run download), put `node` and `codex` on your
`PATH` — hi-agent detects and uses them automatically.

### Runtime install & versioning

The Node and `codex` versions are pinned in
[`src/runtime/manifest.toml`](src/runtime/manifest.toml); the `codex` CLI
dependency tree is pinned by the committed
[`src/runtime/package.json`](src/runtime/package.json) /
[`src/runtime/package-lock.json`](src/runtime/package-lock.json). On first run hi-agent
downloads the pinned Node release from nodejs.org (extracted with the system
`tar`) and runs `npm ci --omit=dev` against the committed lockfile into an OS
cache dir, marks the install complete, and reuses it on every later start.
`build.rs` stamps the pinned versions into the binary; `hi-agent --version`
reports the crate version alongside the runtime component versions (bundle id,
node, codex).

### Voice (optional, additive)

Speech-to-text and text-to-speech are independent capabilities. Each is off by
default; enabling either is a one-provider switch. Both happen to use
Volcengine in this release; swapping either is a single file under
`src/voice/`.

| Variable | Default | Purpose |
|---|---|---|
| `STT_PROVIDER` | `none` | `none` → `POST /audio` returns 501. `volcengine` → enable transcription. |
| `TTS_PROVIDER` | `none` | `none` → `speak(channel="audio")` returns an error string (the agent retries with text). `volcengine` → enable synthesis. |
| `VOLCENGINE_STT_APPID`, `VOLCENGINE_STT_ACCESS_TOKEN` | — | Required when `STT_PROVIDER=volcengine` |
| `VOLCENGINE_STT_CLUSTER`, `VOLCENGINE_STT_MODEL` | sensible defaults | Optional STT tuning |
| `VOLCENGINE_TTS_APPID`, `VOLCENGINE_TTS_ACCESS_TOKEN` | — | Required when `TTS_PROVIDER=volcengine` |
| `VOLCENGINE_TTS_CLUSTER`, `VOLCENGINE_TTS_VOICE`, `VOLCENGINE_TTS_ENCODING` | sensible defaults | Optional TTS tuning |

STT and TTS having separate credentials is deliberate — each capability is
self-contained, so one can be moved to a different provider without touching
the other.

CLI flags:

| Flag | Default | Purpose |
|---|---|---|
| `--port` | `12358` | Loopback port. Everything on this machine reaches the agent here, ungated |
| `--off-box` | *(unset)* | Also accept from off this machine, on `ADDR` (e.g. `0.0.0.0:12359`). Gated: a surface must present a credential. Also `HI_AGENT_OFF_BOX` |
| `--data-dir` | `./data` | Where `journal.jsonl` / `intents.jsonl` / `mcp.sock` live |

Two listeners, because which one accepted a request is what decides whether it is
gated — a single `0.0.0.0` socket cannot tell loopback from the world. See
[`docs/arch/topology.md`](docs/arch/topology.md#auth).

## Project layout

```
hi-agent/
├── Cargo.toml                              # crate + dev-dependencies
├── build.rs                                # embeds the SPA, stamps runtime versions
├── Dockerfile                              # multi-stage build (SPA → rust → debian-slim)
├── docker-compose.yml                      # compose layout (illustrative)
├── Makefile                                # build / dev / run / test / docker
├── docs/
│   ├── impl.md                             # architecture and step plan
│   └── risks.md                            # unverified-things register (Step 0 spike output)
├── src/
│   ├── main.rs                             # CLI; re-exec branch for the MCP shim
│   ├── lib.rs                              # `run(Config)` — wires everything
│   ├── types.rs                            # Conversation, Channel, Signal, JournalEntry, Intent
│   ├── server/                             # axum router + extractors + handlers
│   ├── reaction.rs                          # shared queue, worker registry, interruption
│   ├── codex/                              # codex app-server subprocess + per-thread helpers
│   ├── mcp.rs                              # in-process MCP hub + the seven tools
│   ├── memory/                             # journal, intents, snapshot builder
│   ├── heartbeat.rs                        # 1 Hz tick; absolute-intent firing
│   ├── runtime/                            # first-run node+adapter install; pinned manifest + package files
│   └── appearance/                         # web surface (Rust handlers + embedded Vite SPA)
└── tests/
    ├── http_smoke.rs                       # route surface + header rejection + journaling
    ├── interruption.rs                     # #[ignore] — needs codex, see body
```

## Development

Two processes — the Rust binary on `:12358` and the Vite dev server on `:12359`, with Vite proxying channel routes to `:12358`:

```sh
make dev
```

(That backgrounds `cargo watch` and `npm run dev` with a `trap` so Ctrl-C stops both. Output from the two processes is interleaved without prefixes — if that bothers you, run them in separate terminals.)

The browser talks to `:12359`. HMR works for the SPA; Rust reloads on file change via `cargo watch`.

## Docker

```sh
docker build -t hi-agent:dev .
```

On first run the binary installs its own runtime (downloads the pinned Node and
`npm ci`s the pinned `codex` CLI into a cache dir), so the image needs
no separate agent container. First run therefore needs network access and
the system `tar`. The image still needs `AI_API_KEY` supplied at
runtime for cognition to work.

The image publishes the **off-box** acceptor, so nothing it serves is answered
without a credential. The first run prints one:

```
first-boot surface credential — pair with it once, then revoke it in Settings
```

Present it as `Authorization: Bearer <credential>`, or exchange it once for a
session cookie at `POST /api/session`.

## Risks and known unverified things

See [`docs/risks.md`](docs/risks.md). The headline item: concurrent `codex app-server` sessions have not been measured under load. Validate the concurrency assumption (drive concurrent inputs from several windows and compare wall-clock) before trusting the architecture in production.

## License

MIT. See [`LICENSE`](LICENSE).
