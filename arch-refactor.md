# arch-refactor — the work

**Scratch file. Untracked on purpose; delete when the refactor lands.**

> Historical work log. Its earlier `text_bus` queue/cursor investigations record
> superseded implementations, not the current contract. The authoritative decision
> is [`docs/arch/core.md`](docs/arch/core.md): text is one backend-owned current
> appearance state with no message identity, client identity, cursor, or catch-up.

- **Design lives in [`docs/arch/`](docs/arch/)**. It is the **goal state**, not a description of
  what exists. Never edit it to match the code. If a decision genuinely needs *changing*, change
  it there and note it here — that has happened twice and both times it was the right move.
- **This file is only the work**: what is done, what is next, in what order, and why that order.

---

## How to work on this — read this before deciding to ask something

**The whole point is to move the implementation to the design. Expect the diff to be enormous.**
Hundreds of things do not match yet. Every one of them is *known*, *expected*, and *not news*.

**So: implementation lagging design is not a finding. Just fix it.** Do not stop to report that
Reaction drives Deliberation by a direct host call instead of the one verb, or that a session is
opened with the wrong role, or that a prompt still names a mechanism that was retired. Those are
the work. Reporting them back costs a round-trip and buys nothing, because the answer is always
"yes, align it".

**Surface exactly three things, and decide everything else yourself:**

1. **A conflict in the design.** Two parts of `docs/arch/` that cannot both be true — like the
   address space being `{session id, scene}` while Cognition is defined as having neither, and a
   hop between them being specified anyway.
2. **A missing piece of the design.** Something the code must decide that `docs/arch/` never
   says: who opens Cognition and when, what the task schema is, what a hand-up carries.
3. **A fork you are about to take that changes the design itself** — where aligning would mean
   editing `docs/arch/`, not just the code. Say what you're changing and why, then do it.

Anything else — pick the option most faithful to the design, write down why in the code, and
keep going. A wrong call that is *written down* is cheap to reverse; a round-trip is not. If you
find yourself drafting a paragraph explaining that the code does X but the design says Y, stop
writing and go change the code.

**Corollary on scope.** "Align this rung" routinely means touching prompts, tool surfaces,
registry, loops and tests in one commit. That is normal here, not scope creep. The thing to
avoid is not *size* — it is a commit that leaves something half-aligned and reads as done.

---

## Constraints you inherit

1. **Builds go through `ssh macmini`.** This Linux box has no `cargo`. The Mac mini clone is
   disposable. Build a scratch worktree there off `origin/main` and apply a patch; symlink
   `src/appearance/web/node_modules` from the main clone and `mkdir` an empty `web/dist` or
   `RustEmbed` fails. SSH there is intermittently flaky — pin a `ControlMaster` socket.
   **Build each commit standalone**, not just the final state: an intermediate that does not
   compile is useless for bisecting, and that gap has been shipped once already.
2. **The debt is behavioural, not structural.** `main` compiles and passes **476 lib + 30
   integration** tests, 0 warnings. **Almost nothing has been run against a live instance** —
   the exception is the render path, which drives a real browser in `tests/render_smoke.rs`.
   That gap is the whole of the remaining debt, and every landed rung grows it.
3. **Working agreement:** one fresh worktree per task off `origin/main`; commit when done; push
   `<branch>:main` after the user's go; remove worktree and branch in the same step.

---

## Breaking the app is allowed. Half-finishing it is not.

**The refactor is huge, and the target is the final clean shape — not a working app at every
commit.** Do not preserve a behaviour merely because it currently works; if the settled design
says it goes, it goes. Polish, live-testing and the fix-up pass come **after** the shape is
right, and they are cheap then. Contorting the design to keep the app runnable mid-refactor is
expensive forever, because the contortion is what survives.

Three things this permits, and one it does not:

- **Delete rather than deprecate.** A compatibility path kept "just until the replacement
  lands" is the thing that quietly becomes permanent. Every retired mechanism on this page was
  retired by deletion.
- **Land a rung before the rung it talks to exists.** Machinery that is real and unexercised is
  an acceptable intermediate state; machinery that is *described* and absent is not.
- **Accept a temporarily wrong owner** where the right one has no code — but say so in the
  prompt and in this file, with the name of the item that takes it back. `deliberation.md` held
  the task pen "for now" until N3 collected it; that loan opened and closed exactly as written,
  which is the pattern to copy. The one still open is `any_host`, marked for Reflection.

What it does not permit: **a change that reads as finished and is not.** If something is on
loan, wrong-shaped, or unverified, that fact belongs in the code comment and in this file
before the commit lands. The cost of this whole refactor has been paid, over and over, in
mechanisms that looked done — a dead soul seed, a write-only verb, a switchboard with no
readers, a frame log that kept nothing. Every one of them typechecked.

---

## What the design settled (2026-07-29 → 31)

The interface between every layer was designed and written into `docs/arch/`. The shape:

> **world → Reaction → Deliberation → Cognition → workers**, and results return the way they
> came. Every hop is host-mediated. No agent reaches past its neighbour.

Decisions that overturned earlier ones — each is now in `docs/arch/`, and each cost a round of
being wrong first:

| Decision | What it replaced |
|---|---|
| **One verb: `SendMessage(to, message)`** | `delegate`, `ask`, `surface`, `handoff`, `notify`, and a pull-only worker channel — all were this verb wearing a name that described one use of it |
| **The arbiter is retired into Reaction** | a host module arbitrating a mouth that one per-scene agent already owns |
| **Deliberation is retired into Cognition** | its *reason* (the voice cannot read) was real and moved; its *scoping* was per-scene, and once scene went it was a singleton in front of a singleton. Two rungs meant two hops for one answer. Safe only with its replacement rule below |
| **Cognition never grinds** | what "one session held free for the conversation" was really buying. A brain that dispatches every artifact, side effect and long errand stays as free as a rung reserved for it, in one hop instead of two |
| **A hand-down is a reply owed** | Deliberation's answer was must-relay *structurally* (the host framed the report). Cognition's is mail, and its prompt says mail is a proposal — so the host marks the answer to a hand-down as owed, or a mute-by-default voice is entitled to drop the reply the person is waiting for |
| **Cognition is sole ledger writer and sole worker creator** | two writers to one ledger means one is wrong with no way to tell which |
| **Perception needs no tool** | `see` retired — a ref is a path, and an agent that reads files can open it. What it needs is knowing where things land |
| **Full frames, not modelled events** | modelling existed so the host could infer intent; intent is now explicit |
| **`_meta` can restrict built-in tools** | "ACP cannot do this" — wrong. The spec standardises no allowlist, but `_meta` is the sanctioned extension point and the pinned adapter reads it |

### Changed *since* the docs were written — `docs/arch/` was edited, deliberately

Three. The first two because the design as written could not be implemented as written; the
third because live testing showed the implemented choice was wrong. Each is in `docs/arch/`
now; this is the record of *why*, which the doc does not carry.

| Change | Why the original could not stand |
|---|---|
| **The three thinking rungs are long-lived; the host compacts them.** Reaction, Deliberation and Cognition each hold one session from creation, bounded by the heartbeat swap. Reflection stays per-pass. Reverses the per-wake Cognition session of `d90156f`. `core.md`'s invariant moves from "disposable" to **"replaceable"**; `agents.md` gains a *Session lifetime, per rung* section, because it previously specified lifetime **nowhere** — which is exactly how `core.md` (heartbeat bounds a long-lived session) and the implementation (reopen per wake) drifted apart with both readings defensible | The per-wake rationale was sound on paper and wrong in practice. Journey 05, live: Cognition armed a recurring check, the session ended, the next wake had no memory of arming it, read its own ledger entry warning the check was fragile, and **deleted it as redundant** — declaring an imaginary "central ledger" durable in its place. The ledger cannot prevent this: it records what is **owed**, never what has been arranged, tried, or ruled out. Continuity of *work* is not a fact projection can supply. The original objection (a long-lived session that dies mid-turn wedges silently) is answered rather than dismissed — a failed turn drops the session, a timed-out swap discards it — and N5 comes back onto the path, where it now runs live for the first time |
| **An address is a session id. Nothing else.** Reachability is projected into each rung's window per turn (`Registry::reachable`) | `foundation.md` said "addresses are session ids or scenes" *and* defined Cognition as having neither *and* specified `Deliberation hands up to Cognition`. Those three cannot all be true. Naming a scene was also **retrieval** — the agent guesses a string and cannot tell a cold scene from a wrong name — and this codebase already holds that retrieval is the wrong shape for exactly that failure (invariant 4). A short-lived `Address::Rung(Role)` was tried first and deleted |
| **There is no worker "pool" to re-home** | `registry::global()` already *is* the process-wide session pool; the per-scene map duplicated four of its fields and could disagree with them. Warm reuse — the thing a pool is *for* — had exactly one caller, and it was Deliberation, which is a single id the scene already tracks. Cognition hosts its own workers instead; the map stays for the scene |

**Retired vocabulary — do not reintroduce:** arbiter, delegate, ask, surface, handoff, notify,
spawn, see, WorkerId, ToolCallStub, FollowMailbox, `Address` (any form), scene-as-address.
`any_host` survives for Reflection alone and goes when Reflection gets an inbox reader.

---

## Done and on `main`

| | |
|---|---|
| *(this branch)* | **Deliberation is retired into Cognition.** Four rungs become three. `Role::Deliberation`, `deliberation.md`, `deliberation_prompt`, the `is_deliberation` flag threaded through spawn/drive/report, `warm_deliberation`, `deliberate`, the `deliberation` MCP arm and the per-conversation warm-up all go. The hand-down is now one `post` into Cognition's standing inbox — no session to open, no warm one to resume, no fallback spawn — and `follow_up` died with it, taking the warm-idle path that had exactly one client. `Worker` drops to two handles (`session`, `drive`); its `task`/`transcript`/`busy` were copies of switchboard state whose last reader was the voice's status line, which now asks `registry::session_of_role(Cognition)`. Two jobs moved intact: opening refs, and writing the conversation's brief. One was rebuilt: must-relay, as `LoopInput::Mail { owed }`. **720 tests green, 0 warnings.** Not run against a live instance |
| `d78b350` | **W0 compile pass.** Ten blind commits: 0 errors, 0 warnings. The one red test was stale (`STT_PROVIDER` after BYOK), not breakage. The two feared parallel-authorship collisions never happened |
| `c110aff` | Per-scene rung renamed **Deliberation**. The *cognition tunables* keep the word in its other sense |
| `a1c0a75` | **The scene's memory has a writer.** `memory/prompts/scenes/<id>.md` had a reader since `bcf9781` and none in `src/` |
| `6eeb73f` | **Session identity + ownership.** Process-wide ids; workers record their creator |
| `473252e` | **Reports travel up** to the owner, never sideways into a scene |
| `7a8b4d4` | **`say` is a tool again, and "nothing else" is enforced** — built-ins off at session open via `_meta` |
| `e50b4a5` | `docs/arch/` rewritten to the settled design |
| `95eb389` | **The registry**, standalone with tests |
| `1a8e9c0` | **The one verb, wired.** Sessions register; `send_message` on every rung; `create_worker` on sceneless rungs only; status/messages split |
| `1744398` | **N1 — the character reaches a live window.** The seed had to be re-cut by rung; it was not the reconnection it looked like |
| `47b7f90` | **The render path, driven by a real browser.** `tests/render_smoke.rs` — resolve → launch → CDP → screenshot; `#[ignore]`d |
| `00e420b` `9e6ae45` | **N2 — the one verb gets a receiving end, and the old channel goes.** Mail carries its sender; one mailbox, with readers |
| `f3324de` | **The switchboard stops needing a scene.** `create_worker` belongs to the sceneless rungs and was unreachable by exactly them |
| `70479a9` | **N7a — full frames.** The stream is recorded verbatim *and* kept; the tap was a 4000-frame ring that survived nothing |
| `c3cf110` | **A run has a name.** Frames file per agent session under a run id — session ids restart at 1 every boot, so without it two runs share one file |
| `c085e29` | **The frame log is not a scene, and every session carries its minted id.** Four bugs of one shape. `raw/sessions/` was read back as a conversation and warmed a full loop every boot. Reaction *and* Reflection were opened with `session_id: None`, so the one verb reached only two of four rungs — the voice held a mailbox it had no identity to send from. Reflection was not on the switchboard at all. `create_worker`'s only fence was Reaction's missing id, i.e. an accident |
| `15391e9` | **The observatory admits sceneless events.** `record` takes `Option<&Scene>` and returns *before* the map entry is created — passing a placeholder was enough to put `*consolidation*` in the dashboard's scene list. `SceneView.workers` deleted: it could only ever hold the workers incidentally hosted in that scene |
| `598a243` | **The one verb is observable, and the event log has a reader.** `MessageSent` for agent→agent *and* host→agent, so work travelling **up** is visible for the first time. New Events tab subscribes an `onEvent` callback that had sat with no subscriber since it was written |
| `12fddee` | **The worker map is an optimization, not a pool.** N3a cut — see the design-changes table above |
| `fe47609` `603c365` | **A sceneless rung can be named** (superseded by `9d7bc97`), and **two worker bugs that needed no Cognition**: `{scene}` was substituted raw into `memory/raw/…/file/` where the directory is percent-encoded, so for every `user@device` scene the file-filing worker was sent somewhere empty; and `create_worker` registered *after* a subprocess spawn while telling its caller to message the id immediately |
| `d90156f` | **N3 — Cognition.** Registration is process-lifetime, the ACP session is **per wake** (⚠️ **reversed** — see the long-lived-rungs entry below). Owns the ledger, hosts its own workers under `*cognition*`, never speaks |
| `9d7bc97` | **An address is a session id, and reachability is projected.** Also: one mail renderer, replacing three that had already drifted |
| `fdc4111` | **The clock goes**, and everything that waited on it says so — a task's `due` fires nothing |
| `42c1530` | **The voice's brief is one file**, named for the rung that reads it: `speaking.md` → `reaction.md`; the alarm instruction goes with it |
| `a0aae29` | **A worker has a type**, and the type is a prompt file — `CreateWorker(type)`, five bundled prompts, the const dies |
| `50c0863` | **The view reviewer gets a way to render** (`review_view`) — the first caller the browser stack has ever had |
| `0b8fde0` | **Deliberation is opened as Deliberation**, and the `_` fallback arm goes |
| `5e1780a` | Reaction stops being handed a roster it neither owns nor can act on |
| `3ca29b9` | **Reflex is deferred** — the loop is open at the *authoring* end |
| `dba6c68` | **Reflection is the inward brain** — Cognition's shape, its own worker host; **`any_host` deleted** |
| `fc5f091` | **The context budget gets a writer**, so the session swap can fire at all |
| `8461cde` | The outage notice is a bundled view, and recovery takes it down |
| `a782bad` | The window drops `self.md` and `hot.md` for the brief a rung wrote itself |
| `cd008a6` | The agent is told where its own sessions are kept, and why it copies a handed file |
| `f3961e1` `85f1116` | **Every prompt is one whole file.** The seed, `core.md`, `meaning.md`, `self.md`, `workers/common.md`, `appearance.md`, `aesthetic.md` all deleted; 9 flat files remain |
| `fdc4111` | **The clock goes, and everything that waited on it says so.** See the struck-out N4 — settled, not pending |
| `42c1530` | **`speaking.md` → `reaction.md`, and the frame moves into the file.** A file per role, per `arch.md#character`. The ~40-line Rust preamble (one self; your two tools) was the half an operator could not override. The voice also stops being told to set an alarm it has no tool for |
| `a0aae29` | **A worker has a type, and the type is a prompt file.** `CreateWorker(type)` was in `foundation.md` and not in the code, which is *why* one prompt carried five specialisms as `When your task is to…` conditionals. Now `prompts/workers/` — `common.md` + one per type; the `const &str` is gone, so every role prompt is operator-overridable. **New: `decision-maker.md`, `file-filer.md`** |
| `50c0863` | **N7b (half) — the view reviewer can render, so it can exist.** `review_view(ref, theme?, region?, size?)` on the worker surface: compile → render → verdict + problems + screenshot. 1000+ lines of browser-proven stack had no caller because no session could reach it. **New: `view-reviewer.md`**; `view-builder.md` corrected (it pointed at `look`, which screenshots the *user's screen*) |
| `0b8fde0` | **Deliberation is opened as Deliberation, and the `_` arm is empty.** The rung was opened as `SessionRole::Worker`, so the registry called it one thing and its `X-HI-Role` another — and the fallback arm was not just dead but *armed*: `deliberation` had no arm, so constructing the role would have landed it there with `say` and no `send_message` |
| `5e1780a` | **The voice stops being handed a roster it neither owns nor can act on.** `## Working sessions (delegated)` listed other sessions' workers and told Reaction to `delegate with worker:<id>` — the last live advertisement of a retired tool, generated per turn, which is why the prompt sweep never caught it. Now one line about Deliberation, only while it runs |

---

## The wire changed: ACP + Claude Code → `codex app-server` · **built on `feat/codex-app-server`, 567 lib + 34 integration green, 0 new warnings — not pushed**

**This is a design change, and `docs/arch/` was edited to match** (the ACP vocabulary in `arch.md`,
`core.md`, `agents.md`, `surfaces.md`, `foundation.md` is now "the agent wire" / "one agent session").
`agent-client-protocol`, the node adapter and the bundled `claude` are gone; `foundation/acp/` is
deleted and `foundation/codex/` replaces it. The seam above it — `AgentLayer::session(role, id, opts)`
→ prompt / cancel / `SessionUpdate::{Text,Thought,Frame}` — is unchanged, which is why the rungs
needed renames and nothing else.

**What the swap actually buys, in order of weight:**

1. **A rung's prompt is now the session's system prompt.** `thread/start.baseInstructions` — verified
   on the *upstream* wire as the request's `instructions` field. ACP had no system-prompt slot, so
   every rung's character was first-user-turn content underneath Claude Code's coding-agent persona.
   `vendors/anthropic_messages.rs` existed only to escape that, was never called, and is deleted.
2. **No node between us and the model.** Node survives as esbuild's host, nothing more.
3. **Structured turn errors** (`codexErrorInfo`, incl. `httpStatusCode`) instead of string-matching.
   The 402 gate still reads the message text — the status is appended to it rather than replacing the
   classifier, so the energy edge did not have to be rewritten in the same commit.
4. `thread/resume` exists, and is deliberately **not taken** — threads open `ephemeral: true`, exactly
   as the ACP path opened fresh sessions per boot. That is the obvious next thing if the resumption
   work wants it.

**What it costs, once:** codex has no `disableBuiltInTools`. `exec_command` / `apply_patch` are always
in the schema, so the Reaction's tools-off voice is now soft — a `read-only` sandbox plus
`speaking.md` as the real system prompt. The escape hatch, if the voice reaches for a shell in
practice, is to take it off the agent process entirely (one direct Responses call, no `tools` array),
not a hard rail. This is the one deliberate regression.

**Four things only a live run could have found** (all fixed, all pinned by a test):

- **MCP tool results carry images through to the model.** Spike-verified *on the upstream payload*
  (`{"type":"input_image","image_url":"data:image/png;base64,…"}`), not by asking a model what it saw.
  This was the gate for the whole change — `look`/`see`/`watch` depend on it.
- **`turn/start` returns immediately**; the turn ends on the `turn/completed` notification. Reading
  completion off the response, ACP-style, would have made every turn look instant and empty.
- **A message that never streams still has to be spoken.** Only projecting text from
  `item/agentMessage/delta` made a non-streaming upstream produce `reply_chars=0` on a turn that had
  plainly succeeded. `item/completed` is now a fallback, de-duplicated against the deltas.
- **Codex gates our own MCP tools, even under `approvalPolicy: "never"`** — and it asks via
  `mcpServer/elicitation/request`, not an approval method. A blanket `{"decision":"accept"}` for every
  server request drew `missing field 'action'`; declining the elicitation turned a `say` into "user
  rejected MCP tool call" and the voice went silent. Now: `default_tools_approval_mode: "auto"` on our
  server block, plus an answer *per request shape*, and **an error rather than a guess** for anything
  unrecognised.

**Verified live** on an isolated `--data-dir`, driving a real turn against a stand-in upstream (the
broker's bootstrap is down — `parsing bootstrap response`, **identically on `origin/main`**, so
managed credentials were unavailable to either build): text in → reaction turn → `say` over MCP →
hi-agent's own tool answered. The per-rung tool surfaces come off one `/mcp` endpoint by header alone,
observed on the wire: reaction `{say, send_message, show}`, cognition `{send_message}`. SIGTERM leaves
**zero** orphaned `codex` processes.

**Not verified, and named:** no real model has run a turn. Which models songguo serves on
`/v1/responses`, and whether one is the tier cognition needs, is open — as is the broker bootstrap
failure itself, which predates this and belongs to whoever owns the broker.

---

## Next, in dependency order

### T — Topology: core, app, community

`docs/arch/topology.md` (`530ef8a`) is design-only. The implementation plan is four
phases, ordered so each is worth having on its own rather than by what the doc lists first.

| | | |
|---|---|---|
| **T0** | the `core` → `host` rename | **done**, `cf69e06` |
| **T1** | the gate: two acceptors, the credential, the session, pairing | **done**, `cab4162` — 651 lib + 44 integration green, 0 warnings, **and live-verified** |
| **T2** | the app: a roster and a local proxy | **done**, `feat/topology-app` — live-verified |
| ~~**T2b**~~ | ~~the room~~ | **cut 2026-08-12 — there is nothing to restore; see below** |
| **T3** | the community: registry, then relay + tunnel + the subpath prefix | registry written (site repo), unpushed |
| **T4** | post (push), and refusing to route for a surface reported lost | not started |

**T1 delivers the directly-public shape end to end and needs no Go work**, which is why
it is first: a core in Docker behind a domain is reachable and gated today. T2 is what
makes a *second* core addressable at all; T3 is the largest single item (the tunnel);
T4 is the only part that needs the community to hold state about a surface.

Decisions taken while planning, each one a place the doc specifies a property and no
mechanism:

- **The tunnel is yamux over WSS, and control is not tunnel traffic** — register /
  claim / renew are ordinary HTTPS calls. Written into `docs/arch/topology.md`, with
  the reason reversed HTTP/2 loses (extended CONNECT, and `Upgrade` is exactly what
  remote mic and camera capture ride).
- **The app ships first as a module in this binary** (`src/app/`, T2), not as a second
  process. `CLAUDE.md`'s Phase 1/2 sequencing says do not flip process ownership yet,
  and the seam is the same one the Swift shell will hold later. The local core is
  simply roster entry #1 — host-and-client-are-capabilities is preserved.
- **Registry and relay live in the existing Go binary**, in their own packages with
  their own tables and no shared key with the broker. Handles are subpaths of the
  origin that already ends in an SPA catch-all on `/`, so one mux is the only place
  the reserved-path rule can be mechanically enforced. Invariant 2 is about keys and
  required links, not processes.
- **A fresh `core_id`, not `credentials.device_id`** — that one is the broker
  bootstrap seed, and reusing it would make the address quietly depend on the account.

#### Addressing is a subpath · **settled 2026-08-12, and the doc no longer hedges**

`hi-agent.xyz/ana`. One certificate, no wildcard issuance, no per-handle DNS.

It was changed to subdomains and reverted (`f26f52f`, `37d87c0`) — recorded because the
failure is reusable: I raised subdomains as *a choice to re-open* and then took it on an
adjacent "sounds good". Overturning a settled decision needs the decision.

The reserve apparatus went with it, on the user's instruction — *"not Not in scope, but
please fix the design, and just delete things we don't use."* What was deleted:

- **"Subdomain addressing, held in reserve behind the named trigger"**, and the trigger
  itself. The shared origin is now stated as a property with its real scope — *through an
  app the browser holds nothing*, so the shared jar exists only for the owner pointing a
  browser straight at their own core — rather than as a caveat waiting to be escalated.
- **"Credentials today, keys later."** The keypair roadmap is gone; what stands is that the
  credential is an opaque random token and, in the relayed shape, the community is trusted
  not to replay what it forwards. Invariant 3's caveat says exactly that instead of naming
  keys as the upgrade path, and **invariant 1 was rewritten**: "the community never signs as
  a person" described signing machinery that does not exist and is not planned, so it now
  says what is true and testable — the community is never a principal.
- **A community-held hint list**, **roster sync** (already stated where it belongs, in the
  App section), and **ending TLS at the core** — three speculations nothing was built
  against.
- **"A non-owner interlocutor"**, which argued from an owner the code does not have. Access
  *is* shareable and always was: a credential says which surface may reach a core and never
  who holds it. That now sits in Auth, where access is decided, and says the honest thing —
  sharing shares everything, it is the person's call, and who is *speaking* is a perception
  question the agent answers the way it does in a room.

`## Not in scope` is now `## Not built`, and holds two items instead of seven: mail for a
sleeping core, and a core on iOS.

#### ~~T2b — The room~~ · **cut 2026-08-12**

`topology.md` was the only place it existed. **Scenes are gone from the code**
(`grep -ri scene src/` is 0) and from every other arch doc; `attachments.rs` answers one
question, whether a speaker is attached, and nothing arbitrates a capture slot. So
building "one room at a time" would have been *new* machinery dressed as restoring
something — the shape this file exists to catch.

With scenes gone, most of the invariant is already true: one conversation, unlimited
windows. What is left of it is only *two mics at once*, and a hard slot is the wrong
answer to that anyway — the host would be picking a winner and telling the loser it
lost, where this codebase's answer to "who is speaking" is soft evidence the agent
weighs (voiceprints, faces). The room is removed from `topology.md`: the decision row,
the section, the workflow and invariant 7 all go, and the invariants renumber.

The `## Attachment` section keeps the state table and loses one more retired claim
while it is open — it said ambient absence is "what presence already models", and
presence was removed on 2026-08-11. Nothing is lost by being away because the
conversation keeps, which is the transcript's answer, not presence's.

(`host.md`'s five mentions of `Scene` are *not* stale — they are the deliberate record
of what replaced it. Left alone.)

#### T2 — The app · **on `feat/topology-app`**

There was no app. One binary was core *and* face, the face was served same-origin by
the core it rendered, and there was no roster, no credential holder, and no way to be
with a core that is not this one.

Now: `src/app/` — a roster in `config.db` (`base_url, credential, label`, exactly the
doc's triple), and a loopback proxy in front of it. **The face talks only to the app**
(`--app-port`, 12357 on a desktop install; the tray's "Open" points there), and the
app forwards to whichever core is attached, adding the credential. Three things follow
and the third is the point: the webview never holds a credential; switching who you are
with is the app repointing its proxy with no face involvement; and desktop and mobile
can run identical face code.

- **The local core is roster entry #1**, seeded on first boot with no credential —
  loopback is not gated, which is what makes hosting-and-attaching one act rather than
  a pairing dance with yourself. Host-and-client stay capabilities of an instance.
- **The app exchanges the credential for a session once and caches it**, per entry.
  That is what keeps the long-lived secret off the wire, and it matters in the relayed
  shape where the community terminates TLS.
- **WebSocket passes through** by bridging frames rather than splicing bytes: splicing
  would need a TLS client of its own for the relayed shape, and nothing we serve
  negotiates a subprotocol. This is what lets a remote surface hold a mic.
- **`Set-Cookie` is not forwarded down** — the core's session is the app's to hold, and
  handing it to the browser would put a credential in the one place the design keeps
  them out of.

**One thing T1 got wrong, found by running it:** the app is not a browser, and the CSRF
rule was written as though every cookie-bearing request were one. A forwarded
`POST /api/in/text` (`text/plain` + the session cookie) drew `403`. The app now asserts
`X-HI-Surface` on its own traffic — a proxied request cannot be a cross-site simple
request, because the browser talked to the *app* and the app constructed what goes
upstream. Better there than in the face, which should not have to know the rule.

**Live-verified** on the Mac mini, two cores at once (2026-08-12): core A with an app,
core B off-box and gated. Observed — the roster seeded with "this machine", uncredentialed;
`GET /` and `/api/tools` answered through the app; pairing with B using its first-boot
credential, which B logged as `surface session opened`; attaching to B and posting text
that landed in **B's** conversation while A's stayed as it was; `/api/out/text` through
the one app URL showing whichever core was attached (B even answered, "Core B, I'm
here."); attaching back; and a WebSocket to `/api/in/audio/stream` bridged through to B,
whose `memory/raw/audio/` then held the bytes.

**Not done here:** the roster has no UI yet — it is `POST /api/app/roster` and nothing
renders it, which is Settings' job and lands with T2b.

#### T1 — The gate · **on `feat/topology-auth`**

`auth/mod.rs` said it in its own header — *"hi-agent has **no access gate**… whoever
can reach the URL can use it"* — while `lib.rs` bound `0.0.0.0`. So the Docker shape
was open to whatever could route to it, and "reach a core that runs elsewhere" had no
answer to give.

**Two listeners, because trust is structural.** A single `0.0.0.0` socket cannot tell
loopback from the world, and which acceptor took the request is the whole decision
(invariant 6). `--port` binds `127.0.0.1` only; `--off-box` / `HI_AGENT_OFF_BOX` is a
second socket, unset by default. One router, served twice, differing by an `Acceptor`
extension the listener adds — which **fails closed**, so forgetting the layer costs
access rather than granting it. No IP allowlist: in the relayed shape every request
will share the community's address.

**The consequence is deliberate and it lands on Docker.** A published port is not
loopback, so an existing `docker compose up` is gated from this commit on. That is the
design working, and the first-boot credential — logged once, only when an off-box
listener exists — is what keeps it survivable.

Three things worth keeping in mind when the next rung touches this:

- **SHA-256, not argon2id**, with the reason in a comment. A 32-byte random credential
  is not guessable, so a slow KDF buys nothing and costs latency on every attach. The
  broker's argon2id is correct for what it hashes; someone will try to "fix" this to
  match it.
- **CSRF is narrower than "must be JSON".** A cross-site *simple* request can only
  carry `x-www-form-urlencoded`, `multipart/form-data` or `text/plain` — that is the
  entire exposure, and `X-HI-Surface` is the way through for the two routes that
  legitimately use them. Only the cookie needs it; a bearer is never ambient.
- **Revocation drops the sessions too**, not just the row. Half of it would leave a
  revoked phone working until its session lapsed.

**Live-verified** on an isolated `--data-dir` on the Mac mini, both listeners up
(2026-08-12) — which is the thing this file keeps recording as missing. Observed over
real sockets: loopback `200` and off-box `401` on the same route; the pairing page on
an HTML navigation; `/healthz` open; the first-boot credential accepted as a bearer;
`POST /api/session` returning a `hi_surface` cookie that then works; `403` on a
cookie + `text/plain` POST and `202` with the header; a pairing code minted from
loopback and spent off-box, appearing in the device list under its label; and
`DELETE /api/surfaces/{id}` turning a working credential into a `401`. SIGTERM left
zero orphaned `codex` processes.

**Not covered by T1, and named:** the `Secure` cookie attribute is set only when the
request actually arrived over TLS — always setting it would make a plain-HTTP off-box
deployment silently keep no session at all, which is a worse failure than the one it
prevents where there is no TLS to protect. And `POST /api/pair`'s QR half is not
built: the code and its URL are returned, but nothing renders them yet, because the
surface that would show them is T2's Settings.


### ~~N1 — Revive the soul seed~~ · **on `main`**

`identity::load_soul` was **dead code** — built at startup, stored on `ReactionInner`, read only
by `heartbeat::swap`, which can never run (N5). So `core.md`, the task-ledger instruction,
`proactivity.md`, the skills-workshop pointer, the first-meeting welcome and Settings ▸ Language
all reached no live window.

**"Reconnection, not construction" was wrong, and that is why nobody had wired it.** `core.md`
was written for the single monolithic mind that both spoke and had hands. Five of its sixteen
sections are built on `say`, `show`, `delegate`, `see` or `alarm` — and no live rung has
any of those *and* a Read. Handing it to Deliberation verbatim would have told the rung that
reads to speak with a tool it does not hold. The seed was dead because it no longer fit
anything; reviving it meant **cutting the character by rung**.

What landed:

- **`core.md` is now the agentic self** — who you are, what you know vs. remember, refs and how
  to open them, files, faces, their computer, handing work onward, what's owed and how it's
  held (with the `facet.md` schema), where you stop and ask, and looking at your own output.
- **`speaking.md` is the voice's whole brief** — it absorbed what only the mouth can act on:
  the transcript format, the exchanges, presenting on screen, the built-in view refs, what
  they can receive, presence, speaking first, energy.
- **`character_seed`** replaces `load_soul`: `core.md` + `meaning.md` + `self.md` + the
  workshop, by absolute path. Wired as the **first of three layers** on Deliberation — who it
  is, then the worker capability guidance, then the role. A plain worker still goes without it.
- **`reaction_system_prompt(data_dir)`** is async and reads the *installed* `speaking.md`, so
  `speaking.local.md` reaches the voice — it never did before — and carries the two things the
  seed held that only the voice can use: the first-meeting cue and the language line.
- **`proactivity.md` is projected**, not fetched, in `snapshot::window`. It is consulted before
  breaking a silence and only the voice can break one, so a path to it was a path nobody could
  follow.
- The dead `soul` plumbing is gone (`ReactionInner`, `start`, `lib.rs`). `heartbeat::swap` now
  re-seeds with the real Reaction prompt — and **passes `builtin_tools: Some(vec![])`**, which
  it did not: one rotation would have handed the voice its built-ins back.

`see`, `delegate` and `alarm` are gone from the prompts along with the retired vocabulary.
`alarm` returns with the clock (N4).

**One thing N1 could not fix: who writes the ledger.** `deliberation.md` carried the write
instruction *for now*, explicitly marked as N3's to take back — one writer, not two, but exactly
the kind of thing that quietly becomes permanent. **N3 collected it** (`d90156f`); a test now
pins that exactly one prompt carries it.

### ~~N2 — Retire the old channel~~ · **on `main`**

It could not be a deletion. **`send_message` was write-only**: `take_pending`,
`record_output` and `finish_turn` all had zero non-test callers, so the verb returned
`"delivered"` into a mailbox nobody read, `session_status` reported every session idle with 0
turns, and `session_messages` always answered "nothing yet". Meanwhile `surface` worked —
it drove a turn with no human input. Deleting first would have removed the only functioning
agent→agent path.

Readers first, then the deletion:

- **A scene's voice reads its mail** — its inbox is a wake reason in the loop's `select!`, and
  mail drives a turn. That is what makes `send_message(to: scene)` *reach* someone.
- **A worker reads its mail** — its private `FollowMailbox` is gone. Two mailboxes for one
  session meant the sender's choice decided whether a message was ever read, and only one had
  a reader.
- **`Message` carries `from: Option<SessionId>`** — `Some` for another agent, `None` when the
  host posted it. `post` sits beside `send` because the host is not an agent and is the thing
  that enforces the addressing rules, so it is not subject to them.
- Then `ask`, `surface`, `delegate` — tools, `SceneControl` variants, `Question`/`Surfaced`
  report kinds, and the two observatory events only they fed.

**And the switchboard stopped needing a scene.** All four registry calls sat below a per-scene
sink lookup while three of them touch no scene. `create_worker` belongs to the sceneless rungs;
Reflection runs under the sentinel scene `*consolidation*`, which has no loop — so the one rung
holding the tool was the one rung that could never call it. Same wall in front of the one verb:
a sceneless agent could be sent to but could not send. `create_worker` also now returns **a
session id**, per the contract; it returned a fire-and-forget ack down a one-way channel.

Three bugs surfaced on the way, all latent because nothing drained an inbox:

- `create_worker` was **declared with no dispatch arm** — it answered "unknown tool".
- `open_reaction_session` registered a **new** Reaction per session open and never unregistered:
  three clear-paths on `origin/main`, no `unregister` anywhere, so a scene accumulated voices
  and `Address::Scene` resolved to an arbitrary dead one. Now scope-bound (`register_scoped`,
  released on drop) — the first fix covered two of three exits, which was the same bug smaller.
- The `_` fallback arm's tools (`delegate`, `alarm`, `see`, `record_reflex`) were never
  reachable by any live role.

### ~~N3a — The worker pool leaves the scene~~ · **cut from scope 2026-07-31 — there is no pool**

**The premise was wrong, and it was mine.** "The worker pool is per-scene and must be re-homed"
assumed there is a pool. There isn't: **`registry::global()` already is the process-wide session
pool** — keyed by `SessionId`, holding `role`, `scene`, `owner`, `task`, `busy`, `turns` and a
bounded output tail. Every field of `Worker` except its `JoinHandle` is a second copy of that,
and two copies of one fact can disagree. Building a `SessionPool` on `ReactionInner` would have
been **building a second switchboard**.

What the map is for, taken one at a time:

- **Warm reuse** (`follow_up`) — the thing a pool is *for*. It has **exactly one caller**:
  `deliberate()`. A worker made by `create_worker` is never followed up; it runs once and ends.
  So the entire `WORKER_IDLE_TTL` warm-idle machinery serves one client, and that client is
  **Deliberation** — one long-lived session per scene, already tracked in the single
  `deliberation: Option<SessionId>` field. A map is not needed to hold one id.
- **Reaping** (`reap`) — bookkeeping, and the only reader of `Worker::drive`. The session that
  finished is the thing that knows it finished; self-removal at the end of `drive_worker`
  replaces both and also works for a worker with no scene loop to reap it.
- **Status** (`render_status`) — the only genuine reader, and it wants metadata the switchboard
  already has plus a transcript tail the switchboard already keeps.

**So on-demand creation is fine.** Spawn, register, self-remove. A spawn needs *dependencies*
(memory, agent layer, observatory, views dir), not a home. The map is an optimization that
currently optimizes nothing — it goes when the code next comes through, not as a scheduled step.
`TODO`s left on `WorkerRegistry` and `ToolRegistry::any_host`.

**What is genuinely broken here is provenance, not topology — and it belongs to N3.** All of it
is latent today: `create_worker` is offered only to the sceneless rungs, and Cognition doesn't
exist, so nothing creates a worker this way at all. It bites the moment N3 lands, which is
exactly when to fix it:

1. **`any_host()` lends an arbitrary scene, and hosting leaks into provenance.** The borrowed
   scene becomes the worker's `X-HI-Scene` header (so `watch`/`see` resolve to a stranger's
   camera), the `{scene}` in its prompt, and the scene its report is journaled under — which
   then feeds *that* scene's episodes. The doc comment claims the scene is not told; it is told
   three ways. Fix: take the origin scene as an explicit argument, as the reflection tools
   already do.
2. **`create_worker` instructs the model into a race.** It answers "brief it with `send_message`"
   *before* `register` runs — registration is after an `agent.session().await`, a subprocess
   spawn — so a model that obeys immediately gets `Delivery::Unknown`. Mint, register, *then*
   spawn.
3. **The `{scene}` file path has never existed.** `memory/raw/{scene}/file/` interpolates the raw
   scene, but the directory is `layout::encode_scene` — so for every `user@device` scene the
   file-filing worker is pointed at a path that isn't there.

Still true and still worth doing whenever the pool is touched: **Reaction's "Working sessions
(delegated)" block should lose its worker lines** (it owns none) while keeping one Deliberation
line — `render_status`'s stated purpose is surfacing deliberation's progress, so deleting the
whole block would drop the one thing it is for.

### ~~N3 — Cognition~~ · **on `main`**

**On `origin/main` as of `9d7bc97`, 477 tests green, 0 warnings — never run live.**
`fe47609` the address · `603c365` two worker bugs · `d90156f` Cognition · `9d7bc97` reachability.

The expectation going in was "a properly equipped agent with a system prompt". About 60% of the
words, about 15% of the risk — `cognition.md` is the largest artifact, and four things could not
be a prompt, each of them a way for the rung to exist and not work:

- **An address.** The docs specified the hop (`Deliberation hands up to Cognition`) and never
  the address; the space was `{session id, scene}` and Cognition has neither. Resolved by
  **deleting the address space down to one form**: an address is a session id, and who you may
  reach is *projected* into your window each turn (`Registry::reachable`). Scene-naming was
  retrieval — the agent guesses a string and cannot tell a cold scene from a wrong name — and
  this codebase already holds that retrieval is the wrong shape for exactly that reason
  (invariant 4). `docs/arch/foundation.md` updated to match. ~~**Reflection is still unreachable**~~
  — **not a defect; closed 2026-08-04.** Nothing addresses Reflection and nothing is meant to:
  no prompt names it as a recipient, and it wakes on its own backoff clock. "Unreachable" was
  measured against a general reachability rule it was never a client of. Do not re-file this.
  The original note read:
  nothing projects its id, because nothing drains its inbox.
- **A drain.** A registered rung nobody reads is a mailbox that forgets. The registration is
  created *synchronously in `start`* before the task spawns, or boot has a window where the
  address the prompts now name resolves to nothing.
- **A window.** `snapshot::agent_window` — the projected ledger plus what the agent wrote for
  itself, and none of `window`'s four scene-shaped sections. Invariant 4 exists because a missed
  duty is a silent broken promise, and the *writer* of the ledger is the one that can miss one.
- **A host for its workers.** It registers its own `ToolSink` under `*cognition*`, so
  `create_worker`'s scene lookup succeeds and `any_host()` is never reached. Not the per-scene
  map moving (cut, correctly) — the map is untouched and Cognition has its own.

**The session is per-wake; only the registration is process-lifetime.** `core.md` says sessions
are disposable and continuity lives in `data/`. A long-lived one that dies mid-turn leaves a
handle failing every later prompt with nothing above it to notice — `drive_worker`'s shape,
survivable only because a TTL closes it. Per-wake makes that unrepresentable, and takes **N5 off
the critical path entirely**: context rot has nowhere to accumulate.

**The pen moved**, and what replaced it in `deliberation.md` is deliberately stronger than what
it removed — Deliberation used to record the duty itself, so "you may hand up" would have been a
regression wearing a handover's clothes. A test pins both ends: exactly one prompt carries the
instruction.

**One trap caught in review, worth remembering:** `encode_scene("*cognition*")` is
`%2Acognition%2A` — which `is_pseudo()` does not recognize and `is_scene_dir` accepts. Journaling
under the pseudo-scene, or passing it to the observatory mirror, would have re-created the
phantom-scene bug `c085e29` just fixed, by a different road. Nothing journals under it, and
worker events now pass `mirror_scene()`.

**Not built, on purpose: no boot turn.** The post-restart sequence starts "the clock fires", and
that is N4. Gating on `open_tasks()` being non-empty fires forever for any serving task — the
self-feeding shape `NON_ACTIVITY_CHANNELS` exists to prevent — and would read as restart-recovery
working while a task due in an hour still has no wake.

### ~~N4 — The clock~~ · **removed 2026-08-09. Settled — do not reopen it here or in `docs/arch/`.**

There is no host scheduler. The host's timing surface is the three loops that already exist —
the **pulse**, the **reflection backoff**, and **Cognition's glance-up** — and everything past
that the agent arranges with the shell it already has: a cron entry, a `launchd` job, a parked
worker that sleeps and messages home. `alarm`, `schedule_alarm`, `Alarms`, `take_due` and
`tasks::due_before` are all deleted; no code path fires at a named time.

What that costs is stated once, in [`docs/arch/core.md#glancing-up`](docs/arch/core.md), and
nowhere else: a `due` is read and ordered, never fired, so a deadline is met at the next glance;
and nothing wakes the voice when a promise is running **late**. The safety property that makes
an agent-chosen mechanism sound is `verify` being a **result** check, which lives with
[Tasks](docs/arch/data.md).

### N4′ — Cognition's own wake, and liveness in the projection · **built on `feat/task-resumption`, 502 lib + 30 integration green, 0 warnings — not pushed, never run live**

**The hole N4 named, hit by L2 on real hardware.** A standing duty ("帮我盯着油价") was
accepted, correctly written to `facets/tasks/oil-price-watch/facet.md`, and then the host was
restarted: two pulses fired, both turns silent, **zero workers respawned, Cognition woken 0
times**. Asked how it was going, the agent said it was still watching — while its own facet
read `being set up`. Gap 1 kills the duty quietly; gap 2 makes sure you never find out.

**The owner was never the open question.** `agents.md` specifies the sequence and
`cognition.md` has carried the prose for it since before anything could deliver a pulse to
that rung. What was missing was the wake: `select!` had mail, control, reports and shutdown,
and the pulse woke the two rungs that *cannot* read the ledger.

- **Cognition paces itself** — `BOOT_WAKE_AFTER` (30s, so recovery does not race a
  half-stood-up process) then the `pulse` cadence, both gated on the ledger being non-empty.
  Safe against N3's objection because the boot arm is one-shot and the recurring arm is
  timer-paced: an always-open serving task re-arms the timer, it never re-enters it.
  `pulse: off` silences the cadence and **not** the boot wake.
- **`pulse_interval` / `render_pulse` are shared** with the scene loop — one knob, one
  `(pulse)` vocabulary, and dropping `pulse` for a journey now speeds up *every* wake there
  is rather than all but one.
- **`Task::checked` is projected.** `line()` rendered kind/due/title/`report_to` and dropped
  `Liveness` entirely, so a watch that never started, one that died, and one running
  perfectly were byte-identical — which teaches the tools-off rung that existence means
  health. That is gap 2's whole mechanism, and it typechecked for months. Appended *after*
  the per-line clip and counted in the summary tail: the bound may hide a task, never that
  nothing has confirmed it.
- **`reaction.md`** gains the matching half — report the line, never vouch for what you have
  not been told is alive, set a check moving instead of reassuring.

`docs/arch/core.md#clock` status line updated (precedent: `fdc4111`). **The design did not
change** — this is implementation catching up to `agents.md` and to `data.md`'s "liveness is
a contract, not an existence check".

**Still missing, and it is exactly `At(_)`:** a task's `due` fires nothing, so a deadline is
met at the next glance rather than on time, and nothing wakes the voice when a promise is
running *late* — the other half of the L1 finding, untouched.

**Unverified:** both wakes are timing-dependent and have no test (`note_for` pins the
empty-ledger gate and the note wording only). The re-test is the same journey: accept a
standing duty, restart the host, watch for a Cognition wake and a respawned worker within
`BOOT_WAKE_AFTER`, then ask how it is going and check the answer against the facet.

### N4″ — Arrival: the duty inbox · **built on `feat/reactive-duty-inbox`, 726 lib green, 0 warnings — not pushed, not run live**

**N4′ gave a duty a heartbeat; this gives it a doorbell.** A `serving` task was reactive
at its edge and cadence-paced at ours: the Feishu listener received a message the instant
it was sent, appended a row, and nothing read that row until the next glance — up to a
pulse later. Real-time in, half an hour to notice.

`POST /api/in/duty/<start_key>` closes it. The listener says what arrived; a working
session handles it; **Cognition is not in the path**. `start_key` was already in
`Liveness` with nothing reading it, and is now the one durable name a duty and its
machinery share — session ids are minted per boot and nothing durable may hold one.

- **`body/reaction/duties.rs`** — the inbox. Coalesces per key (settle = the reaction
  loop's own `RESPONSE_SETTLE`, shared not copied; a 30s floor as the cost ceiling; a 5s
  cap so a trickle cannot push the settle forward forever, with the floor outranking the
  cap). Resolves the key against the ledger, posts to the live handler, or opens one with
  **the facet as the brief**. Owner is Cognition so an escalation has an address; the
  per-message terminal report goes to a drain, which is that decision expressed as a
  channel rather than as a flag threaded back through `spawn_inner`.
- **The handler is a cache, not the carrier.** key→session lives in memory and is never
  written down; a `post` returning `Delivery::Unknown` is not an error but the signal to
  re-derive from the facet. This is what makes it safe to have a per-duty session at all
  — the objection to one is that a session cannot hold a duty, and this one does not.
- **It gives `drive_worker`'s warm-idle path a client again.** That path lost its only
  caller with Deliberation (`fbad7d3`), and it is exactly right here: a burst continues in
  one session with full context, a message the next morning opens a fresh one.
- **The nudge is not the truth.** The listener's append-only rows stay the record and
  `verify` still reads them on the cadence, so every failure on this path — saturated
  inbox, closed handler, energy pause, restart — degrades to the behaviour that existed
  before it. Hence `try_send` at the door and a drop rather than a wait.
- **The ledger authorises.** A key no `serving` task claims is dropped: this is a door for
  reaching a session the ledger says should exist, not for making one. The routing key is
  never interpolated into a prompt or a path (pinned by a test).
- No new `Channel` variant, and nothing touches the transcript — `host.md`'s
  "`/api/in/text` is not a wake channel" is honoured by not going near the conversation.

`docs/arch/host.md` gains the third row and the constraints above; **this is a design
change, stated rather than made quietly** — the two-shapes table said cadence covered the
standing duties this system actually has, and arrival is a third shape it did not have.

**Unverified — and it is the whole point:** no live run. The journey is Feishu-shaped:
plant a `serving` facet with a `start_key`, POST a burst to the route, watch one handler
open from the facet and one turn take all of it; then let it idle past the TTL and POST
again to see it re-derive; then restart mid-burst and confirm the glance-up still finds
the unhandled rows. Nothing pins the floor or the settle against a real clock.

### ~~R — Role prompts~~ · **on `main` (`a0aae29`), then superseded by the flattening (`85f1116`)**

`42c1530` the voice's brief is one file · `a0aae29` a worker has a type.

**Two prompts that named tools nobody holds are gone.** `speaking.md` told Reaction to
*"set an alarm"* — a tool declared only in the dead `_` arm, for a clock that no longer
exists as of `fdc4111`. `WORKER_SYSTEM_PROMPT` told workers to *"call the `ask` tool"* —
retired with the old channel. Both promises survive with honest mechanisms: the voice
sizes a silence and leans long *because nothing will remind it*; a worker raises a
question to its owner and never waits.

**`speaking.md` → `reaction.md`, and the Rust frame moved into it.** `arch.md#character`
says a file per role and this was the one named for an activity. The ~40-line `format!`
preamble above it — the one-self framing, the two-tool brief — was the half an operator
could not override, so the voice's character lived in two places with one editable.

**`CreateWorker(type)` is real, and that is what made the split possible.** The type has
been in `foundation.md` since the interface was written; the tool took only `task`, which
is *why* the const had conditionals (*"When your task is to file a file…"*) — with no
type there is no way to hand a worker a different prompt. Now: `prompts/workers/` =
`common.md` + one file per type (`general`, `view-builder`, `decision-maker`,
`file-filer`), composed base‹layer, each half with its own `.local.md`. The const was the
last role prompt that was not a bundled `.md`; the smaller-items row for it can go.

**`docs/arch/agents.md` took two corrections**, both settled with the user, both
narrowing rather than changing intent — *"may spawn further workers"* → a worker fans out
with **sub-agents inside its own session**, invisible here (no id, no address, no
registry entry), so `CreateWorker` stays Cognition's and Reflection's and one-dispatcher
survives; and the Decision Maker is reached **through the owner**, which is one of those
two rungs anyway. `foundation.md` won the contradiction because it was right.

### ~~N7b′ — The view reviewer needs a **tool**, not a prompt~~ · **on `main` (`50c0863`)**

Discovered while writing the prompt, and it changes the size of the item. `view_render`
is a *bundled capability* precisely so a reviewer does not improvise a browser per
install — its module doc says so outright — but **nothing exposes it to an agent**. So a
`view-reviewer` worker has no way to render, and a prompt telling it to would be
machinery described and absent.

The slice: `resolve_view_ref` → `ViewCompiler::compile` → module url →
`view_render::render` → return the PNG as an image content block (the `do_look` pattern,
`mcp/mod.rs:835`) alongside `problems`/`blank`/`verdict`. Plumbing is the only unknown:
`ViewCompiler` lives on `ReactionInner` (`reaction/mod.rs:607`) and has to reach
`dispatch_tool`. Then `WorkerType::ViewReviewer` + `workers/view-reviewer.md`.

That is N7b, and it lights up `view_render.rs` (417 lines), `chrome_headless.rs` (642)
and `/render/view` — all browser-proven by `render_smoke.rs`, all with zero production
callers.

### ~~L1 — The live-test pass~~ · **first run done 2026-08-02 — twelve firsts, one finding**

Fresh `--data-dir` on the Mac mini: boot → one turn → a real errand. Verified from ground truth
(transcripts, `server.log`, artifacts), never from what the agent claimed.

**Green, all first-ever:** boots clean with both sceneless rungs registering synchronously ·
**`say` adopted** (`mcp__hi-agent__say` in the transcript — the #1 silent-failure risk, gone) ·
`show` + the first-meeting welcome · the character actually Read · **the scene brief
written**, and it reads as a brief · the frame log fills under a run id · Deliberation opened as
`role="deliberation"` · Reflection firing on its own clock under the sentinel · Cognition woken
by a hand-up · **a worker spawned under `scene=*cognition*`**, which *is* the `any_host` deletion
proven — a stranger's scene would have hosted it before · **the ledger written**
(`memory/facets/tasks/uv-research-card/facet.md`, carrying `state`, `report_to`, an episode ref)
· **`budget_chars: 9219`**, a counter that had been structurally frozen at zero.

**The finding: work completes and the person is never told.** The errand finished, the ledger
recorded it, and the answer surfaced **only when the boss asked again** — after a promised
"一两分钟" and ten minutes of silence. It was ready and waiting, so the mail path works. Three
layers, and only the middle is a disobeyed instruction:

1. **Reaction obeyed.** `reaction.md` says speak when the work lands; nothing landed. Its own
   fallback is "your next quiet moment" — the pulse, at **30 minutes**, against a promise made
   in minutes. Nothing in the host fires at a named time, so that mismatch is permanent.
2. **`cognition.md` is weakly worded** — message the scene *"if there is something a person would
   want to hear"*: a conditional with no bias, and no notion that a scene which handed up is
   *waiting*.
3. **The promise never travels, and this is the real hole.** Reaction made it, Cognition decides
   when to report, and Cognition **cannot know it was made** — `agent_window` deliberately
   carries no scene-shaped sections and the hand-up carries the work, not the expectation. The
   ledger is our mechanism for durable promises and it holds the *duty*, not the *promise*.

Fix: Deliberation carries the expectation with the hand-up — it reads the room *and* hands up, so
this is already its job — plus sharpening `cognition.md`. **Not yet written.**

**It also reopens a fork closed the same morning.** "Does `WakeTarget` need a scene?" was closed
*no — the pulse is a scene-loop timer, not a clock client*. Correct about the pulse. A promised
check-in is a third registration, inherently per-scene, targeting **Reaction**. The "running
late" half cannot be fixed softly at all: nothing can wake the voice to say so.

**Half of that is now answered, by presence rather than by a clock (N6).** A *return* is
per-scene, targets Reaction, and needed no timer at all — it is caused by the person, so it is
observed directly rather than polled for. The half that stands is the one that genuinely needs a
clock: nothing wakes the voice when it is running **late**. Being told when they come back is not
the same as being told on time.

**Not a rung — the reason there are no more rungs.** With the clock skipped, nothing left on
this page is blocked by unbuilt machinery: the spine `world → Reaction → Deliberation →
Cognition → workers` exists in code end to end for the first time. That is the trigger
condition this file already wrote down — *"polish, live-testing and the fix-up pass come after
the shape is right, and they are cheap then."* The shape is right. They are cheap now and get
more expensive with every further commit, because each one adds a candidate to the blame set.

The two things at the top of the backlog **fail silently**: an unadopted `say` is a mute agent
with no error, and a ledger nobody writes is an agent that simply never records a promise. No
type checker and no test on this machine can ask either question.

One first meeting on a fresh `--data-dir` on the Mac mini exercises items 1, 1b, 4, 5, 6, 7, 8
and 9 of the backlog at once. Method is in `CLAUDE.md` — terse boss, don't lead the witness,
verify every claim outside the conversation (`server.log`, `GET /api/sessions`, the scene
transcripts, `memory/facets/tasks/`).

Two adjustments the clock's deferral forces on the plan: **journey 25 (resume interrupted work)
now recovers only at the next pulse in a scene**, never on a due time, so test it that way or
not at all; and **the pulse tunable is the only wake there is**, so drop it to ~120s for the
session and reset it after.

### ~~N5 — Session hygiene~~ · **done, and it has now run live**

The counter got its writer in `fc5f091`; the swap reached all three thinking rungs with the
long-lived-rungs change (see the design-changes table). `swap_working` is a sibling of `swap`
rather than a generalization: Cognition and Deliberation seed **no journal tail**, because
everything they need is re-projected into every turn, so the briefing carries only the working
thread — and it asks for that rather than "who you're talking with", which is the wrong
question for a rung that talks to nobody.

**First live run, 2026-08-04** (this whole path had never executed once): with `compact`
lowered for the test, all three fired clean in one session — `reaction session hot-swapped`,
`deliberation session hot-swapped`, `cognition session hot-swapped` — no failures, no wedges,
and the conversation carried its earlier task across the swap.

Still unexercised: a swap that **fails** or **times out**. Both arms are written (keep the warm
session; discard the unresponsive one) and neither has been provoked.

### N6 — Presence actually gating · **built on `feat/presence-gate`, 494 lib + 30 integration green, 0 warnings — not pushed, never run live**

**Two thirds of this item was already done, and the framing above was wrong.** "Rendered only
as prompt text" was true of `presence.rs` and false of the system: `reaction/mod.rs` already
projects `## Presence` into Reaction's window every turn, and `reaction.md` already carries a
whole *How present they are* section plus *Speaking first* — including "Hold the telling for
their return". Reaction was told to gate and had the facts to do it. What was missing was the
mechanism underneath, and it was **narrower and differently shaped** than this entry assumed.

**The design's own justification was false for two of three channels.** `core.md` said an
utterance to an empty room "is spent". Checked: **text keeps** (`text_bus.rs` buffers 32
utterances per scene and delivers to a late-opening GET — it exists to kill exactly that bug),
**views keep** (`view_bus.rs` retains whole state, replays on connect, snapshots across
restarts), **only voice is spent**. So the host-enforced half of the gate is one condition, and
`reaction.md:349` had it right before either the doc or the code did.

What landed:

- **The TTS span is the gate, and the only one.** `open_tts` checked `tts::available()` — a
  global capability question — and never whether *this scene's* speaker is attached. It now
  checks reach. Nothing else is withheld: `show` is explicitly left ungated, with the
  reason in the code, because a view shown to an empty room is waiting when they arrive.
- **`say` stops lying.** It returned the constant `"spoken"` while its own tool description and
  `surfaces.md` both promised the return tells Reaction what became of the utterance. Now
  `Spoken::{Voiced, TextOnly, Held}`, each a sentence about what actually happened. This is the
  described-and-absent shape this file exists to catch, and it typechecked for months.
- **`ToolSink.beats: Option<Sender<Beat>>` → `mouth: Option<Mouth>`**, where a `Mouth` carries
  its sequencer *and* its presence read *and* its scene. The reach has to be read at the instant
  of emission, not from the turn's rendered snapshot — a turn outlives the window that started
  it — and a mouth is a scene's channels, so the two belong in one place.
- **Presence has an edge: `Presence::returns`.** The load-bearing half. Everything else about
  presence is polled during a turn that was already happening; a return happens precisely when
  no turn is happening, so nothing observed it and "hold it for their return" meant "hold it
  until they type", or until the pulse — 30 minutes. `Woke::Returned` on the scene loop,
  `LoopInput::Returned` in the batch.
- **Only a first-party activation counts.** A reconnecting out-channel is not a person:
  `/api/out/text` is a long-poll that re-opens on its own while a tab sits forgotten. The
  attention lane (`visibilitychange`/`focus`) is the one signal that means a human hand. It also
  **self-debounces** — `note_activation` sets `last_engaged`, so the edge cannot re-arm until
  another full `AWAY_AFTER` of silence, which is why a page refresh (disconnect + immediate
  reconnect) can fire at most once. No debounce state, and a test pins it.
- **Suppressed when a reply is already owed** — they typed *and* the page reported focus, a race
  with no fixed order; without this the same arrival gets answered twice.
- **Dropped while the vendor is down**, unlike mail: mail's content keeps, a return is a moment
  and announcing it after an outage clears would be announcing a stale arrival.
- Journaled on `Channel::Clock`, i.e. inside `NON_ACTIVITY_CHANNELS` — a return is *presence,
  not content*. It must not hold a scene warm by itself, nor push one over the frontier
  threshold into consolidating on nothing.

**`docs/arch/` was edited, deliberately — three changes.** (1) Presence's derivation is
**app-observed only**; `core.md` claimed it fused OS idle, screen lock, a face and speech, which
`presence.rs` explicitly refuses and gives the better reason for — those measure "away from the
keyboard" when the question is "away from hi-agent". Face and speech *are* observed and reach the
agent as journaled signals it weighs, which is where soft evidence belongs. (2) The "it is spent"
justification narrows to voice, with the reason both other channels don't need it. (3) The clock's
intended registrations lose **"you just came back"** — a return is not due at a time, it is caused
by the person, and a timer could only find one by polling. `surfaces.md` also loses its last
**arbiter** references, which were retired vocabulary.

~~**Still open, and named rather than papered over:** nothing wakes the voice when it is running
*late* on a promise — the "running late" half of the L1 finding is untouched, because nothing in
the host fires at a named time. A return gets them told; nothing gets them told on time.~~
**Closed by N8** (2026-08-10, on `feat/check-in`, unpushed). It stayed open one N too long:
`Returned` was doing double duty as the progress mechanism, and a return is the moment the
person was about to ask anyway.

### N8 — The check-in · **built on `feat/check-in`, 611 lib + 35 integration green, 0 warnings — not pushed, never run live**

**The "running late" half of the L1 finding, closed — and it was a broken product, not a
rough edge.** Reported by the user from normal use on 2026-08-10 (`b7dc549`, local
`make dev`): *"i have to ask progress often, which is not nice ux"*. The day's
`data/memory/raw/text/2026-08-10/text.jsonl` is unambiguous — three separate "progress?"
from the boss in one morning, each answered completely and instantly, after silences of
**13, 15, 13 and 18 minutes**. The answer was always ready. Nothing sent it.

Three mechanisms, and the third is the one that mattered:

1. **The pulse cannot reach these gaps.** Default 30m, and `last_activity` resets on
   every turn — so no silence in the transcript ever came close.
2. **A worker reports at completion only.** Nothing travels up mid-flight.
3. **The number the voice names had no reader.** `reaction.md` tells it to put a size on
   a silence and then says outright *"You have no timer — nothing taps you on the
   shoulder at the minute you named"* — while the same section's last line is the
   judgment: *"a check-in that arrives is a promise kept; one they have to ask for is
   already late."* The character was right and the host could not keep it.

What looked like proactive updates in the transcript were all riding on `Woke::Returned`
— and a person coming back to the window is precisely the moment they were about to ask.

**`docs/arch/` was edited, deliberately.** `core.md#glancing-up` gains *the check-in* and
loses "nothing wakes the voice when a promise is running late" from its costs;
`surfaces.md` gains `back_in` on `say`; `agents.md` gives Reaction the deadline as part
of the social layer it already owns. **This does not reopen N4.** One slot per voice, one
deadline, one wake, no target, no payload; a task's `due` still fires nothing; scheduling
past a cadence is still the agent's own. It is a second deadline in the `select!` that
already carries the pulse's.

- **`say(text, back_in)`** arms it. On `say` and not a verb of its own because a promise
  is only a promise once it has been *said* — a separate call could arm a wake for a
  number nobody was told. An overlong (rejected) utterance arms nothing.
- **`Said` replaces the bare `Spoken` return**, so the ack confirms the number the host
  is now holding. An unreadable `back_in` arms nothing **and says so** — swallowing it
  would leave the voice believing it was covered, which is worse than no timer.
- **`LoopInput::CheckIn`** is its own wake beside `Pulse` and `Returned`, for the same
  reason `Returned` is: rendering it as `(pulse)` would tell the voice to stay quiet at
  the instant it should speak.
- **A floor under an open-ended silence** (`check_in`, default 5m, doubling to the pulse,
  `off` disables). The observed failure was mostly promises with *no number at all*, so a
  mechanism resting entirely on the model remembering the parameter would have missed the
  majority of it. Deliberation-busy only — Cognition's workers are not this loop's to
  describe. The note distinguishes the two sources: a floor must never claim a promise
  nobody heard. **The backoff dial is whether the last check-in produced speech**, not a
  blind doubling — the voice is the only thing that knows if there was anything to say,
  so a cadence that keeps landing stays at 5m and one that keeps passing in silence
  widens itself out of the way.
- **Discharged by the thing it was about coming back** — a Deliberation report clears an
  untouched slot, so the voice is not woken to say "you told them they'd hear by now"
  right after it has just told them. A slot the same turn re-armed is kept.
- **Dropped into an empty room**, not held: the words would be held anyway and `Returned`
  already wakes the voice with a fresher read.

**Unverified, and it is the whole point:** no live run. The re-test is one real errand
that takes >5 minutes — hand it over, *don't ask*, and watch for `check-in fired` in
`server.log` plus whether what gets spoken is actual progress rather than "still working
on it". `render_status`'s 240-char tail is the only substance the voice has to build that
from, and whether that is enough is exactly what a live run answers.

### ~~N6 (original framing)~~

`src/body/presence.rs` fuses three reach axes properly and is then rendered only as prompt text.
Nothing holds a self-initiated turn while the room is empty. Now unblocked: `say` returns, so
there is somewhere for the decision to live — and per the new design the decision belongs to
**Reaction**, not a separate module. ~~Branch `feat/presence-gated-speaking` has prior work.~~
**That branch does not exist** — not on `origin`, not locally. It was presumably only ever on the
MacBook. Nothing was inherited from it.

### ~~N7a — Full frames~~ · **on `main`**

It was two gaps, not one, and calling it "big" was wrong — roughly ninety lines plus tests.

**On the stream.** `SessionUpdate::Frame(Value)` carries every update whole and is emitted
unconditionally, so a variant this build has never heard of still arrives intact — permanent,
not a gap to close, because the schema enum is `#[non_exhaustive]`. `Text`/`Thought` survive as
convenience projections for the speech path; they no longer decide what is worth keeping.
`ToolCallStub` and `Other(String)` are gone.

**On disk.** The tap was already mirroring every JSON-RPC line, both directions — into a
4000-frame in-memory ring that keeps nothing. The stream fix alone would have left the record
evaporating. It now writes to `memory/raw/sessions/<run>/<session>.jsonl`.

Two corrections landed on top of the first cut, both from the user:

- **Per session, not per day.** A session is the unit that gets replayed, and one subprocess
  hosts exactly one session, so the unit was already there.
- **Under a run id.** `registry::mint` is a process-local counter starting at 1, so without a
  run, today's session 3 and tomorrow's session 3 are one file. `foundation::run` mints twelve
  hex characters at startup and is deliberately not persisted — a run id read back from disk
  would be an install id wearing the wrong name.

The tap now carries hi-agent's minted session id *alongside* the protocol's: the protocol's is
parsed off the line and absent during `initialize`/`session/new`, while ours exists before the
subprocess starts, which is what makes the handshake frames land in the right file.

### N7b — Verification: the reviewer, and replay

**Nothing reads the frame log yet.** The record exists; the consumer does not.

**View reviewer.** `view_render.rs` (417 lines), `chrome_headless.rs` (642), the browser
provisioning stack and the `/render/view` page have **zero callers** — but the browser half is
no longer a guess: `tests/render_smoke.rs` drives real Chromium end to end, including the
failure path (a page that never reports comes back timed-out *and* blank, so white cannot read
as empty-by-design). What is unproven is `/render/view` with a real compiled view, which needs
a running server. Wiring one worker makes all of it live; it is a prompt, not a build.

**Historical messages come from ACP `session/load`**, not a second copy we keep —
`LoadSessionRequest { session_id, cwd, mcp_servers, … }`, gated on
`AgentCapabilities.load_session`. Now that frames are kept verbatim, the old lossy replay path
is gone; what remains is deciding when to reach for `session/load` at all.

---

## Live-test backlog — nothing below has ever run

**This is now the work itself, not a backlog behind it — see L1.** Ordered by what breaks worst
if wrong; one first meeting on a fresh `--data-dir` covers 1, 1b, 4, 5, 6, 7, 8 and 9.

1. ~~**`say` adoption.**~~ **Run 2026-08-10 against the live instance. It was mute — the risk as
   written, exactly.** Four reaction turns in the day's `data/memory/raw/sessions/*/2.jsonl`:
   four `agentMessage`s, **zero `say` calls**. A plain "the google login, is it done?" drew a
   complete 195-character answer that was typed, thrown away by `drive_voice`, and logged as
   `turn done unspoken_chars=195` — a successful turn that reached nobody.

   **The cause was not prompt adherence; it was the tool surface.** The voice was holding
   codex's built-ins (one turn ran `nl -ba views/people/voice-roster.jsx | sed …` mid-sentence),
   and a session with a shell answers like a coding agent: markdown findings with file:line
   links, as message text. Two builds, same model, same first message, fresh `--data-dir` each:
   built-ins on → 0 `say` in 4 turns; built-ins off → `say` from the first turn on.

   Fixed in `voice-never-mute`, two parts: the enforcement below, and a host-side backstop —
   a turn that produced text and no utterance is handed its own miss once
   (`recover_mute_turn`), then logged at ERROR if it stays silent. Note the backstop **did not
   rescue the built-ins-on arm** — it nudged and the model still would not call `say`. The
   surface is the fix; the nudge only catches the stray turn.
1b. **The ledger gets written at all** (N3). Second only to `say`, and for the same reason: the
   pen moved out of `deliberation.md`, so between that commit and a Deliberation actually
   handing up, **nothing writes the ledger**. Failure looks like an agent that simply never
   records a promise — no error, no degradation. Test: a real errand in a scene should leave
   `memory/facets/tasks/<subject>/facet.md`, and the Events tab should show the hand-up edge
   into `cognition` that caused it.
2. **`_meta` tool restriction.** Verified present in the pinned adapter's `dist/`; never
   exercised. If it silently no-ops, Reaction has full built-ins and the enforcement is theatre.
3. **Images through `Read`.** Retiring `see` rests on images reaching the model over ACP. If
   wrong, Deliberation is blind to every photo.
4. **The scene brief.** Whether Deliberation writes it, and whether what lands is a *brief*
   rather than a transcript summary. Doubles as the live check `bcf9781` never got.
5. **Every-turn projection + outbound journal** (`bcf9781`, `2438f3d`) — changed *when* the
   window is injected and *why* a turn is recorded. Neither is falsifiable by a type checker.
6. **The re-cut character** (N1). Two questions a type checker cannot ask: does Deliberation
   actually Read `core.md` when the seed tells it to, and does the voice still sound like
   itself now that `speaking.md` carries twice as much? A first meeting on a fresh
   `--data-dir` exercises both ends of it at once.
7. **Mail drives a turn** (N2). `send_message(to: scene)` is supposed to *reach* the person, not
   sit until they speak next. Nothing has ever sent one to a live scene.
8. **The frame log fills** (N7a). One turn against a real instance should leave a readable
   `memory/raw/sessions/<run>/<session>.jsonl` with tool-call payloads in it. Cheap to check and
   it proves the whole verification substrate at once.
9. **The Events tab shows a real edge.** One turn should leave `message_sent` rows with real
   `to_session` ids — the first time an agent-to-agent crossing has been visible at all.
10. **Reaction's window when the worker lines go** (with N3) — narrower than it looked: only the
    worker lines leave, the Deliberation line stays, because that is what the block is for.
11. ~~**The presence gate and the return wake**~~ (N6) · **first run done 2026-08-05, by accident —
    one finding, and it invalidates the gate's own justification.** Not run as the scripted
    three-question test below; the user hit it in normal use. Window backgrounded, voice muted,
    and the agent produced anyway — then the window was opened onto a reply that began at "二",
    with "一" gone, which reads as an agent that cannot count rather than one that lost a line.

    **The finding: text is *half* spent, and the gate cannot see it.** `core.md` narrowed the
    host-enforced half to voice alone on the grounds that text is deferred — "buffered per scene
    and delivered to a reader that connects later". That holds only while a connected reader
    implies a person. It doesn't: `reach` is derived from which out-channels are *subscribed*
    (`presence.rs`), and the face kept `/out/text` and `/out/audio` subscribed behind a
    backgrounded window. So the buffer handed the utterance to a window nobody was watching and
    deleted it (`text_bus` is drain-and-delete, not a cursor), while the caption band kept
    revealing on a timer that doesn't know whether anyone is looking and evicted past
    `AGENT_REPLY_WINDOW = 3`. **The mechanism that was supposed to defer the words is the one that
    spent them** — and half-spent is worse than either honest end, because a fragment that looks
    whole costs trust in every message that isn't visibly truncated. Sizing the window is the
    wrong layer: any N is wrong while the band advances unwatched, and an N large enough to never
    lose anything makes the face a chat log.

    Two more, found by tracing rather than by running: **speaker mute never reached the wire** —
    it set a flag on the player while the subscription stayed up, so the gate saw a speaker and
    TTS was synthesized, billed and streamed into a muted sink while `say` answered "spoken
    aloud", which is exactly the spend the gate exists to prevent. And **occlusion was invisible**
    — an occluded `WKWebView` keeps `visibilityState === "visible"` and fires no event, so a
    window behind a full-screen app was indistinguishable from one being read.

    **Fixed on `fix/presence-honest-channels`** (536 lib + 30 integration green, 0 warnings, 16
    vitest; `view_render::a_good_view_renders_to_a_non_blank_png` fails identically on clean
    `origin/main`): out-channels are held open only while attended, mute detaches the audio
    channel, `windowDidChangeOcclusionState:` reports the case no web API does, and **reach leaves
    the projection** — it is answered by `say` at the instant of emission, so a second, staler copy
    in the window can no longer disagree with it. Expectation stays, being graded and unlearnable
    from a failed send. `core.md` gains the honesty condition the narrowing always depended on.

    **Still unrun, and now the interesting half.** The scripted questions below were never asked
    and the fix makes them askable for the first time — *does the gate bite the right thing?* (no
    audio client → text on `/api/out/text`, no `AudioBegin` in `server.log`; one attached → both),
    *does `say`'s answer change behaviour?* (the ack is only worth its cost if Reaction holds a
    spoken line rather than repeating it into a room that can't hear — and now that reach is gone
    from the window, the ack is the **only** thing that can teach it, so this stopped being a
    nice-to-have and became load-bearing), and *does the return fire?* (quiet past `AWAY_AFTER`,
    then `POST /api/in/attention`; expect `attention: they're back after an absence` then
    `presence returned; waking the voice`, with `(they're back)` and not `(pulse)`). The failure
    worth watching for is still the opposite of silence: a return that greets an empty-handed
    arrival every time, the "bet that misses" `reaction.md` spends a section warning about.

    **One seam left open on purpose.** A held utterance still enters `text_bus`, so the single
    utterance that *discovers* the absence flushes verbatim when the window returns, alongside
    whatever Reaction recomposes for `(they're back)`. Bounded (Reaction stops calling `say` once
    it reads "waiting") and arguably right — those are the words said as they left — but it is a
    verbatim replay inside a design that decided deferral is *composition*, not delivery. Closing
    it means not enqueuing at all, which needs the outbound log to exist first (`data.md`: "the
    honest gap today is outbound, which is barely recorded") or the words vanish with no trace.

---

## Smaller, independent, any time

| | Where |
|---|---|
| `WORKER_SYSTEM_PROMPT` is a `const &str` in `workers.rs` — the one role prompt that isn't a bundled `.md`, so it alone can't be operator-overridden. It also still describes `ask` in prose, which no longer exists | `reaction/workers.rs` |
| `prompts/` is documented bundled-only; `compose_prompt` still layers a `*.local.md` override | `identity/mod.rs` |
| `RESPONSE_SETTLE` → a tunable; its comment cites a deleted client VAD | `reaction/mod.rs:90` |
| `AcpSession::cancel()` — zero call sites; wire or delete | `acp/session.rs:317` |
| A photo journals without waking the mind, unlike a file | `server/vision.rs` vs `files.rs:94` |
| File arrives as prose with **no locator**, and there is no pull tool for a file ref | `server/files.rs:111` |
| Bulk stored twice: bytes land in the log, then a worker is told to **copy** them into `drive/` — the thing `surfaces.md:66` forbids, by instruction | `memory/media.rs:30`, `workers.rs:134` |
| ~~Nothing suppresses a `show` when no window is live~~ — **resolved as "no gate wanted" (N6)**. A view is retained state: the view bus folds it, replays it to whatever connects next, and snapshots it across restarts. Showing into an empty room costs nothing and is waiting when they arrive. The reason is now in `show`'s doc comment so this doesn't get re-raised | `body/reaction/tools.rs` |
| Vendor recovery is never announced — `note_success()`'s `was_down` is discarded with `let _` | `reaction/mod.rs` |
| The vendor-outage apology is a hardcoded Chinese string **emitted by the host**, not Reaction | `reaction/mod.rs:1589` |
| `record_reflex` sits in the dead `_` arm, so nothing can teach a reflex; the whole recognizer/fire/invoke path operates over a directory that can never be populated. **The `_` arm is now the only thing left in it** — every other tool there was deleted with the old channel, so this is a decision about `record_reflex` alone: give it a live role or drop it | `mcp/mod.rs`, `reflex/mod.rs` |
| Absolute-path sweep — invariant 11 is violated by worker reports, and nothing validates. (`character_seed` is clean: every path in it is absolutized, with a test) | `workers.rs` |
| touch / smell / taste 501 stubs — channels the architecture does not have | `types.rs:64`, `server/stubs.rs` |

---

## Deferred on purpose — do not treat as bugs

- **Closed tasks accumulate.** Nothing deletes them; the cost lands on anything that
  *enumerates*. The answer is in the invariant's wording — never pruned *while open*, so closed
  and cold ones age out like ambient identity clusters.
- **Ping-pong is possible.** Two long-lived agents can message each other indefinitely. Expected;
  guided by prompt, logged rather than blocked.
- **A worker's `Bash` can read the auth token from its own env.** Non-hacker threat model.
- ~~**`_meta` tool restriction is vendor-specific.**~~ ~~**Moot since the codex swap** — Codex offers
  no built-in-tool switch at all, so the Reaction's tools-off voice is now soft guidance plus a
  read-only sandbox.~~ **Wrong, and it cost us the mute voice above (2026-08-10).** Codex has the
  switch; it is spelled as a *permission profile*, not a tool list:

      "permissions": { "hi-agent-voice": { "default_tools_enabled": false } },
      "default_permissions": "hi-agent-voice"

  in the thread's `config`. The flatter spellings do not exist — 0.144.1 under `--strict-config`
  answers `unknown configuration field tools.default_tools_enabled`, and
  `permissions.default_tools_enabled` with `expected struct PermissionProfileToml`. Codex logs
  `Permissions profile 'hi-agent-voice' does not define any recognized filesystem entries …
  Filesystem access will remain restricted` when it takes, which is both the proof it parsed
  and the posture Reaction wants. The MCP attach is configured separately and survives it.
  Reaction only — `agents.md` gives "no built-ins at all" to no other rung.
- **Cross-scene ambient awareness is weaker** than the old global digest. Continuity routes
  through Cognition instead.
- **No hand-edit lever.** Load-bearing — see `docs/arch/arch.md#character`.

---

## Journeys

Seven still reference retired mechanisms (`hot.md`, `commitments.md`, `self.md`):
`03-feishu-flash-cards`, `02-feishu-sprint-backlog`, `05-news-and-watch`, `13-equip-a-capability`,
`24-skill-improves-and-refreshes`, `28-first-meeting`, `README`. Two are already done
(`20-reuse-built-views`, `25-resume-interrupted-work`) and show the pattern.

**Update — the three files are now retired in code too**, so these names no longer point at
anything that exists: no reader, no writer, no path helper. `28-first-meeting` (a live
precondition), `24` (which held `self.md` up as the model for a skill note) and `README`
are corrected. The four left — `03`, `02`, `05`, `13`, plus `gaps.md` — are **dated 实测
records**, where the file name is part of what was observed on the day; those want the
`20`/`25` treatment (a preamble saying the mechanism was replaced and by what), not an
edit to the finding itself. Still open.

Add to the sweep: anything naming `delegate`, `ask`, `surface`, `see`, `alarm`, or the arbiter.

**The rule that matters:** the journeys' *promises* all still hold — only the implementation
prose rotted. Correct the mechanism, never the promise, and prefer naming the concept (the log,
the task ledger, the scene's brief) over a file path, so it rots more slowly.
`28-first-meeting` is the user's own recent work — minimum change.

---

## ~~The two that stopped the run~~ · **both answered 2026-08-02**

- **Reflection is a standing rung** (`dba6c68`). The user's framing sharpened it beyond
  "copy Cognition": the split is not brain-vs-curator but **outward vs inward** — Cognition's
  work arrives from a person and it owns the ledger; Reflection's is the agent's own house and
  *nobody asked for any of it*, which is exactly why it needs a rung rather than being what
  Cognition does when idle. `any_host` deleted with it.
- **`record_reflex` gets neither — the whole reflex rung is deferred** (`3ca29b9`), labelled in
  code, with the note that the loop is open at the authoring end and nobody should quietly
  re-add the tool to an arm to make the module reachable.

The original text of both asks, for the reasoning:

- **Is Reflection a pass, or a standing rung?** `agents.md` corrected itself to say
  Reflection **does** dispatch — and a rung that dispatches must read reports, which
  needs a loop that outlives one prompt. Reflection today is a *pass*: register scoped,
  one `prompt`, wait, drop. So it can create a worker and can never hear back, and
  `heartbeat.rs:338` says so outright ("nothing drains this inbox"), deferring to N3.
  N3 has landed, and its shape is the obvious template — registration process-lifetime,
  session per wake, loop draining mail + control. Adopting it also kills **`any_host`**,
  the last open provenance leak (it lends an arbitrary live scene, which becomes the
  worker's `X-HI-Scene`, the `{scene}` in its prompt, and the scene its report is
  journaled under). What the docs never say is Reflection's **lifetime**, which
  Cognition's section states explicitly for itself. Copying Cognition wholesale is my
  default; it is a restructure of the consolidation pass, so it wants a yes.
- **Does `record_reflex` get a rung, or get deleted?** As of `0b8fde0` it is declared to
  **nobody** — the `_` arm that held it is empty. The recognizer and
  `POST /api/reflex/invoke` are live, so a reflex can be fired but never written. The
  plausible home is Reflection (a taught repetition is close to its `episode → skill`
  graduation), but that is a new graduation and therefore a design edit.

## Open forks, still unanswered

- **Barge-in stop path** — core-owned or forever client-side? Today it is in the browser and
  the backend hook (`interrupts::mark_flush`) is dead code.
- **Per-install prompt instances** — should `data/prompts/` ship knowing whose agent it is?
  Currently no: it learns by meeting them.
- **`journal.recent()` runs per turn**, parsing the day's jsonl. Caching deliberately not added;
  revisit once something can actually run.
- ~~**The clock has no scene-shaped target.**~~ **Answered by the deferral (2026-08-01):** the
  pulse is not a clock client. It is a scene-loop timer today and stays one, so `WakeTarget`
  never needed a scene. If the clock is ever built, it inherits **task timers only** — the
  pulse and the reflection backoff each keep pacing their own loop, which is what
  `docs/arch/core.md` already says about everything that paces a subsystem rather than waking
  an agent.

---

## Where each layer stands

`FOUNDATION` — most built, and as of `598a243` the messaging layer is not only closed but
**visible**: every agent-to-agent crossing is recorded, in both directions, and the event log has
a reader. The session stream is kept verbatim per session under a run, and **the renderer is
proven against a real browser** — it needs a caller, not repair. What is left is one thing, not
two: **nothing reads the frame log**. The verification substrate exists and has no consumer. (The
"worker pool" that used to sit here was not a gap — see the struck-out N3a.)
`DATA` — log, episodes, facets, skills, views, forgetting, and the tasks *reader* all built. The
stores were never the problem, and the loan is repaid: **the tasks writer is Cognition's**, with
a test pinning that exactly one prompt carries the instruction. The ledger now has a second
reader too — `agent_window` projects it to the rung that writes it.
`CORE` — least built, and **on purpose**: the session swap and the clock were not deferred but
**removed**, so the loop paces itself inline and that is the finished shape, not a gap waiting on
a commit. What is genuinely unfinished here is behaviour, not structure: the vendor gate is
half-classified, and one mechanism — `record_reflex` — can never fire. It is labelled where it
lives, which is the standard this file holds. **Presence now gates** (N6) —
the one thing that can be spent is withheld, `say` reports what became of an utterance, and
coming back is an event the voice is woken for. Lateness is now an event too (N8): the voice
names a size in `say`'s `back_in` and the host wakes it when that is up, with a floor beneath
for the silence it left open-ended. It did not want the clock — it wanted one deadline, which
is the thing N4 was never arguing against.
`AGENTS` — all four rungs real **and now wearing their own clothes**: Deliberation is opened as
`SessionRole::Deliberation` with the one-tool surface the design gives it, and the `_` fallback
that would have swallowed it is empty. **Every worker kind is a prompt file**, including the two
that had no code at all — View Reviewer (with `review_view`, so it can actually render) and
Decision Maker. The prompts no longer name tools nobody holds: `ask`, `delegate`, `alarm` and
`look`-as-view-review are gone from them, and `render_status` — the last generated one — with
them. What is left here is one question, not a gap: **Reflection is still a pass, not a rung**,
so it can dispatch and cannot hear back, and `any_host` survives for exactly that reason.
`SURFACES` — **who may reach the core is now decided** (T1): two acceptors, one
credential in two presentations, and the loopback path unchanged. Text and audio solid
end to end. Vision and file journaled but half-connected — no
wake, no locator. Apps and device-as-surface absent.
