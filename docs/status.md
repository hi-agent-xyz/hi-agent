# Status — what is built, what has been watched, what is missing

**This file is the ledger. [`docs/arch/`](arch/) is the goal state.** A design document that
doubles as a status report goes stale in a way that makes readers distrust the design too, so
the design carries no status and this file carries no design. Where they disagree, the design
wins and this file is the bug.

It replaces `arch-refactor.md`, the scratch file the architecture refactor ran out of. **That
refactor is done**: the spine `world → Reaction → Cognition → workers` is in code end to end,
every hop host-mediated by the one verb; the retired vocabulary below is gone from `src/`; and
topology T0–T3d are deployed and live-verified through the CDN. One item is still mid-build —
the task-ledger manager, immediately below. The full log — every rung, every reversed decision
and the reasoning that cost a round of being wrong first — is in git:

    git show 0294cde:arch-refactor.md

---

## The standard this file holds

**Built is not watched.** Nearly everything here typechecks and passes tests; the question that
has repeatedly cost us is whether it has ever run against a live instance. A mechanism that is
described and absent typechecks. So does a dead soul seed, a write-only verb, a switchboard with
no readers and a frame log that kept nothing — every one of those shipped green.

Three states, and only the first is done:

- **watched** — observed against a running instance, from ground truth *outside* the
  conversation (`server.log`, `GET /api/sessions`, the frame log under `memory/raw/sessions/`,
  disk artifacts) — never from what the agent said it did
- **built, never watched** — green tests, no live run
- **no code**

Method for moving something from the second state to the first is in
[`CLAUDE.md`](../CLAUDE.md) § *Testing user journeys live*: terse boss, don't lead the witness,
verify every claim.

---

## In flight — the task ledger gets a manager

**The one item still mid-build.** Design landed 2026-08-19 in `agents.md`, `data.md` and
`foundation.md`; the shape is **create vs change**:

> **Cognition may create a ledger row. Only a Task Manager may change one.**

The split is defensible where "one writer" was not, because the two are different acts: opening
is something Cognition *witnessed* — the ask happened in the conversation it was in — and must be
instant; closing is a claim *about the world*, which the dispatcher is worst placed to make. They
never touch the same row-state, so they cannot contradict each other. Cognition keeps
`CreateWorker`, so one dispatcher survives.

The stamps that follow a status change (`status_since` / `completed_at` / `cancelled_at`, and the
legacy `kind:`/`state:` spellings) are **the store's, not a mind's** — normalised on the diff pass
that already re-reads the whole ledger every turn. Deliberately not a new verb: every agent that
may write this ledger has a shell, so a verb is a door beside an open wall, and its failure mode
is an absent field, indistinguishable from a task that never moved.

**Built:** `WorkerType::TaskManager` + `identity/workers/task-manager.md` (real and unexercised);
`tasks::reconcile`, run from `tasks::projection` — a dry run rewrote 58 records on the first pass
and 0 on the second; `cognition.md` hands closing down instead of doing it.

**Watched failing, 2026-08-19 — it had never once got past the 7th record.** `render` refused any
field quoting this machine's absolute data-dir path, and the refusal aborted the whole loop, so
the 61 subjects sorting after `ai-agent-book-reading-guide` were never reconciled: 18 with no
`status_since` (so the *last moved — close it, ask once, or cancel it* line can never fire on
them), 8 still on the legacy `kind:`/`state:` spellings. It said so once per brain turn, in a
warning that named no task. **A dry run could not have caught it**: the check compared against
the *running* data dir, so a store copied to another path passes clean. The refusal is deleted —
item 2 below already said this pass reports what it cannot fix and never refuses — portability
is guidance in `cognition.md` now, and the reader takes both path forms. **Built, not yet
watched:** nothing has observed the next brain turn actually catch those 61 up.

**Still to build, in order:**

1. **A recently-closed `serving` row keeps its place in the manager's window** — closed when,
   carrying a `verify:`, not checked since. **The most valuable thing left here**, and the one
   that would have caught `feishu-it-group-watcher`: cancelled 60s after a reflection named it
   as on duty, machinery still running a day later, a failed self-heal spawning **461 orphaned
   processes in 25 minutes** (708 MB) with nothing said — because closing had removed the only
   `verify:` from view.

   **Re-specified 2026-08-19, and the correction is the buildable part.** The first wording said
   the store re-runs `verify:` after a close. It cannot. Every `verify:` in the live store is
   prose — bilingual, multi-clause, naming a *result*: *"at least one has been OPENED and looked
   at"*, *"三条齐才算活着"*, *"有一句这个空白到底在哪的判断"*. That is precisely what stops *"a
   job with this id exists"* from passing forever, and precisely what makes the field unrunnable
   by anything but a mind. So the store surfaces the staleness and the manager does the looking —
   the same split as everywhere else here.
2. **The pass reports what it cannot fix** — misfiled `verify:`, unknown status words, a task
   directory with no row, a closed record with no instant to date it — into the manager's window
   **as facts, never as refusals**.
3. **The manager's own first job**, not a migration script: the 5 legacy records that came out of
   `reconcile` closed-with-no-instant, the 2 task directories with no `facet.md` at all,
   `kt8-070` / `ktv-doubao-ref-only` still in `done` after an audit that found neither had reached
   the person, and `feishu-it-group-watcher` cancelled while its machinery runs.
4. **The no-subject exemption.** `CreateWorker(subject)` is omitted for a manager, but the
   reachable list still renders that as *not linked to any task* — the line that means "staff
   this" — so the manager trips that alarm on itself every glance-up. The prompts say to omit
   `subject`; nothing enforces it.
5. **Nothing starts a manager.** No code path creates one, nothing checks that one ran, nothing
   notices if none ever does — `git grep TaskManager` outside the enum returns nothing. The
   chain is *pulse fires* (code) → *Cognition starts a manager* (prompt) → *manager files*
   (prompt), and two of the three links are prose. Same lever as (1) and it belongs beside it:
   the window carries *"N tasks past the idle boundary; no manager has run since X"*. Cognition
   cannot derive that by thinking harder, which is the test for earning a place in a window.

**And one floor that is not a step, recorded so it stops reading as an oversight.**
`checked_at` — the one liveness field code reads — can only ever be written by a mind. A
manager that stamps it after a probe that came back *down* makes a dead duty read healthy,
which is `feishu-it-group-watcher`'s own failure mode, and no pass can tell the two apart from
the outside. What the store can do it already does: show *when* it was last confirmed, so
staleness is visible even when honesty is not verifiable.

---

## Watched working

Against running processes, not by reading code.

**The spine.** Boot with both sceneless rungs registering synchronously · `say` adopted from the
first turn · `show` and the first-meeting welcome · the character actually read · the frame log
filling under a run id · Cognition woken by a hand-up and hosting its own workers · the task
ledger written · Reflection firing on its own backoff · the session swap on all three
long-lived rungs · Cognition resuming a standing duty after a host restart · the duty inbox
(`POST /api/in/duty/<start_key>`) taking a burst into one handler.

**Topology.** The local shape; the directly-public acceptor; the relayed shape end to end
through the deployed community and Tencent EdgeOne; the gate in both credential presentations,
with CSRF, all three pairing paths, the device list and revocation; the app's roster and local
proxy across two cores; a name claimed by a signed-in account, permanent, refused to a second
account and to an anonymous one, surviving a wiped data directory, and released back; the
subpath prefix, emitting its own base and serving through the tunnel; `_builtin/reach`; the
roster screen at the app's own `/app`; and **the relayed page rendered in a real browser** —
React mounting, views resolving through the prefixed import map, SSE reconnecting.

**The privacy boundary was rebuilt around a much smaller subject, and the live run above no
longer describes it.** It was a projector on every serialized Responses request, reached by
pointing codex's provider at a loopback proxy inside hi-agent. The subject is now *one inbound
human message*: `POST /api/in/text` files what a person typed, and `AgentSession::prompt`
substitutes the file's path. The proxy, the per-boot proxy token, the `shell_environment_policy`
exclusion and the whole-request projection are **deleted**; codex talks to the provider directly
again over `HI_AGENT_LLM_KEY`. Two defects the proxy had shipped went with it — a header
allowlist whose effective forward set was two headers, and a `redact-core` byte-window panic on
Chinese text that unwound out through the axum handler. The ASCII stand-in that fixed the second
is kept, because detection still runs.

What the rewrite gives up on purpose, per [`privacy.md`](arch/privacy.md): tool results, the
system prompt, agent-to-agent mail and **codex's own shell** are all untouched. A session that
runs `cat` on a secret file gets the value and nothing stops it. The guarantee is against the
person's accident, not against the agent's decision.

**The four host readers are gone with it.** `hi_read_text_file`, `hi_read_journal_range`,
`hi_read_session_log` and `hi_copy_file_to_drive` existed to keep a model-authored command
away from `memory/raw/`, which stopped being a goal when the boundary narrowed to inbound
text. Each was a narrower version of something the shell already does — a `cat` with a
1 MiB cap, a `jq` over `uuidv7` ids that sort lexicographically anyway, an `ls -t | head -1`,
a `cp` with three guardrails the design explicitly does not want. `{raw_dir}` and
`{sessions_dir}` are back in the prompts, so a ref is a path again, as
[`agents.md`](arch/agents.md) never stopped saying. `journal::range` went too — its only
caller was one of the four. Net: four tools and their prompt prose out, two string
substitutions and one sentence about `keep/` in.

**Built, never watched — all of it.** Nothing in the new shape has been seen running. The unit
and integration tests cover ingest filing, the marker, restart stability, PII being left alone,
a key inside Chinese text, and the broker spending a filed key. Nobody has watched a real turn
carry `⟨secret: …⟩` into a live model, nobody has watched a worker build a `cat`-based
command from one, and nobody has watched a rung open a ref by path since the placeholders
came back — including the `keep/` fallback for a faded original, which is the one case a
bare path does not cover. The re-test is the 2026-08-19 journey re-run: paste a key, confirm the
conversation still shows it and the log still holds it, then ask for it to be used against a
real endpoint.

**The cache-control rule, re-measured against the real edge.** A gated response carrying
`public` was an auth bypass: one authorized fetch taught EdgeOne the body and the edge then
served it to requests carrying nothing, with the core never seeing them. `401 MISS` where it was
`200 HIT`. The rule is `topology.md` § *Nothing behind the gate is `public`*, enforced **at the
core, not the relay** — a cache-control rewrite at the community would be the second
authorization mechanism invariant 3 exists to prevent — and pinned by a test rather than left as
a habit.

---

## Built, never watched

Each of these is green and unexercised. Ordered by what breaks worst if wrong.

| | The re-test |
|---|---|
| **The floor** — the mouth refuses a turn whose room isn't the voice's to take (their voice sounding; a line went unheard), with a three-refusal backstop | A real partial stream. Both conditions are unit-tested; curl cannot drive a partial |
| **The check-in** — `say(text, back_in)` arms a wake, with a floor under an open-ended silence | One errand that takes >5 minutes: hand it over, **don't ask**, watch for `check-in fired` in `server.log`, and judge whether what gets spoken is progress or "still working on it" |
| **The duty inbox's edges** — TTL re-derivation, and a restart mid-burst | Plant a `serving` facet with a `start_key`, POST a burst, idle past the TTL, POST again; then restart mid-burst and confirm the glance-up still finds the unhandled rows |
| **A genuinely new machine keeping its name** | Covered by `TestRegistryNameSurvivesANewMachine`, not by a live run — `device_id` is machine-derived, so two data dirs on one Mac share an account and the live run could not tell the cases apart |
| **The Docker shape's gate** | A published port is off-box, so an existing deployment is gated from first run. Reasoned about, not exercised |
| **Nothing opens during the drain** — `AgentLayer::session` refuses once the shutdown signal is triggered, so no rung can mint a thread while the host is winding down | The failure it fixes was watched on 2026-08-20: a cold reopen during the drain wrote a newer `thread` row for Cognition and Reaction, and the next boot resumed those 14-record shells instead of the 34.6 MB thread the run was actually spent on. The guard itself is unit-tested only. Quit with rungs live, then confirm the last `thread` row per rung in `memory/raw/sessions/index.jsonl` is still the run's own, and that the next boot logs `resuming=2` on those same two ids |
| **A session swap that fails or times out** | Both arms are written (keep the warm session; discard the unresponsive one) and neither has been provoked |
| **The conversation surviving every view** | `ViewTraits` and the `.geom.json` sidecar are deleted, so `stage()` no longer has a `hidden` case at all — the popover/pill pair is the whole of it. Covered by unit tests on both sides; what none of them reaches is the outage, which is the one view that ever claimed the words. Pull the vendor credential, watch the notice come up, and confirm the conversation is still there and still typeable |
| **A show taking the window with it** | The parked window follows any show now, including a re-show of the same destination (the cursor drops on the newest history entry changing, not on its destination changing), and the server clears its `attention` on any show to match. Unit tests reach the server half only: park on an older card, have something shown, and confirm the stage moves; then re-show the same view and confirm it still moves. A dismiss deliberately moves nobody |
| **The Sessions tree** — the roster drawn as the ownership tree it is, packed by `d3.tree()` (Reingold–Tilford as Buchheim refined it) against one measured card height every card is drawn to, with one rAF animator owning card positions, the arrows between them, and the fade of a session starting or ending | Checked hard, but never mounted. The pure half (`forest` + `layout` + `fitted`) was run directly against a fabricated roster — ladder order, one rank per depth, no two cards overlapping in a rank, a parent centred over its children, the synthetic root d3 needs never leaking into the drawing, an orphan and an ownership *cycle* each drawn exactly once, and narrowing that stops at the card-width floor. The published-library path was checked end to end rather than assumed: the view's bare `d3-hierarchy` specifier survives the server's own esbuild transform (`--loader=tsx`, no `--bundle`), and `make build` emits it in `dist/importmap.json` against a real 14KB chunk. The drawing was rendered in real Chrome at 980 / 1600 / 2200px in both skins, and the create/end transition sampled frame by frame off the page's own rAF (two cards cross-fade, the ended one unmounts at ~330ms, everything settles at full opacity). All of that was a standalone page carrying the real stylesheet and a stubbed `/api/workers`. **The mounted view has since been seen against a live roster** — three rungs over six workers, the arrows following the `owner` chain — which retires the "never mounted" half of this row. The 2026-08-21 card pass (one height for every card, one-line titles, a live stage a screenful tall, room under the plot for the card's own shadow) was rendered off a running instance's real `/api/workers` in headless Chrome, including the empty roster and a fifteen-card tree; that reading mounted the file directly rather than going through the server's own view pipeline, and the pass has not been seen in the app itself. `make test` still reaches none of this: a factory view has no test harness, and building one is a boundary decision nobody has taken. What no reading has covered is **churn** — watch a worker start under Cognition and confirm it animates rather than jumps, and confirm a tree too wide for the frame scrolls with its edge fades rather than hiding a rung |
| **The views band opening on the bookmarks row** | The upper row's scroll-to-*here* has been watched; the lower row's is new, and its arithmetic was only checked in a standalone page carrying the real stylesheet — not in the mounted band, where the chip appears a render after the first `listViews()` answers. Open the band while parked on a `factory/` surface far along the row and confirm its chip is on screen |
| **Working ahead** — the handover carries the questions it provokes, and the reversible half of the likely next step is handed out in the same turn ([`agents.md` § *Working ahead*](arch/agents.md)) | Journey [34](user-journeys/34-a-step-ahead.md), written to be run without leading the witness. **The one that decides it**: hand over something whose next step is outward, then say nothing, and confirm it stopped at the door. This is prompt-level throughout — the identity tests pin that the prose is present and that the permission never travels without its boundary; nothing pins that the agent acts on either |
| **The `ahead` count** — `hi_create_worker(ahead)` → `WorkerSpawned` → `N/M errands started ahead` in the events view | It is self-reported, so it can only undercount, and a zero is two different findings wearing one number: nothing is being prepared, or nothing is marking it. Read it against the wire frames' actual `hi_create_worker` arguments on a run where working ahead plainly happened, or the count grades its own homework |

---

## No code at all

- **A shelf for prepared things.** Working ahead has no store: a prepared artifact lives
  wherever it naturally lives, the fact that it exists rides the worker's report into
  Cognition's own session, and nothing carries it across a wake or a restart. **Deliberate,
  not deferred** — a store, a budget, an expiry and a ready-line in the window are each
  justified by a failure journey 34 has not shown yet, and machinery that exists before
  anything is being lost is the pattern this whole file is a ledger of. What reopens it:
  a run where prepared work is redone or forgotten, which is exactly what that journey's
  reverse test looks for.

- **post** — the push service, and with it waking a surface. Deliberately not next: push exists
  to wake a surface holding no channel, and a phone browser opening the relayed address needs no
  waking to be useful. The native iOS client changes that calculus; it has not been re-decided.
- **Refusing to route for a surface reported lost** — the one revocation case a sleeping core
  cannot serve.
- **Mail for a sleeping core**, and therefore core-to-core addressing. Nothing is queued; an
  inbound request is answered `asleep`. This is also the trigger that makes keypairs
  non-optional.
- **Credentials in the OS keychain, on the desktop.** The app keeps them in its config store.
  (The iOS client already uses `KeychainStore`; the desktop app has no equivalent.)
- **A core on iOS.** Blocked by the wire being a spawned binary, not by effort. (`app/apple/ios`
  is a *client*, not a core.)
- **`At(_)`** — a task's `due` is read and ordered, never fired, so a deadline is met at the next
  glance rather than on time. This is a deliberate cost, stated once in
  [`host.md#glancing-up`](arch/host.md), not a gap waiting on a commit. See *Settled* below.
- **The retention question.** `data.md#keys-passwords-and-the-one-question` still describes the
  one-time *this / all / none* choice; ingest files every detected secret automatically.
  Because a prompt describing an unbuilt question teaches the agent to claim an answer was
  applied, the prose was **removed** from `reaction.md`, `cognition.md` and
  `drive-organizer.md` rather than left to rot, and
  `identity::tests::prompts_are_honest_about_current_auto_retention` now pins the opposite: both
  rungs must say retention is automatic and the choice is not implemented. Exchange-scoped
  temporary secret files go with it.
- **Any filtering outside the two seams.** Tool results, the system prompt, mail, exported
  diagnostics, and codex's own shell output are all unfiltered, and
  [`privacy.md`](arch/privacy.md) says so as a scope decision rather than a gap: only text a
  person typed is the subject. Nor is anything non-text — a key spoken aloud or inside an
  uploaded file is not detected.

---

## Loose ends

Small, independent, none blocking. Verified against `0294cde`.

| | Where |
|---|---|
| `record_reflex` is **declared to no role** — the recognizer and `POST /api/reflex/invoke` are live, so a reflex can be fired but never written. Reachable by name only. This is a decision waiting, not a bug: give it a live role or drop the module | `foundation/mcp/mod.rs:972`, `body/reflex/mod.rs` |
| `RESPONSE_SETTLE` is still a `const`, not a tunable — and it is now shared by three readers (the batch, the floor, the duty inbox), which raises the cost of it being unadjustable | `body/reaction/mod.rs:126` |
| Vendor recovery is never announced — `note_success()`'s return is discarded with `let _` | `body/reaction/mod.rs:1950` |
| `interrupts::mark_flush` has **no caller outside its own tests** — the barge-in stop path is client-side, and the backend hook is dead. Wire it or delete it | `body/reaction/floor.rs:497` |
| A stale parenthetical: *"(Vision only journals; a handed file must wake the mind.)"* — vision **does** wake now, on a presence change | `foundation/server/files.rs:20` vs `vision.rs:188` |
| The inspect views still list `deliberation` as a rung | `appearance/web/src/inspect/{SessionsView.tsx,api.ts,inspect.css}` |
| touch / smell / taste 501 stubs — channels the architecture does not have | `types.rs:65`, `foundation/server/stubs.rs` |
| Journeys naming retired files (`hot.md`, `commitments.md`, `self.md`): `02`, `03`, `05`, `13`, and `docs/memory.md`. These are **dated 实测 records** — the file name is part of what was observed on the day, so they want the `20`/`25` treatment (a preamble saying the mechanism was replaced and by what), not an edit to the finding. **Correct the mechanism, never the promise**, and prefer naming the concept over a path so it rots more slowly | `docs/user-journeys/` |

**Unverified in this pass, carried forward rather than dropped:** whether worker reports still
violate the absolute-path invariant, and whether bulk media is still stored twice (the
`file-filer` worker that carried the copy instruction is gone; `drive-organizer` replaced it).

---

## Open forks

- **Barge-in stop path** — core-owned or forever client-side? Today it is in the browser and the
  backend hook is dead (above).
- **Per-install prompt instances** — should `data/prompts/` ship knowing whose agent it is?
  Currently no: it learns by meeting them.
- **`journal.recent()` runs per turn**, parsing the day's jsonl. Caching deliberately not added.
- **Replay** — the frame log now has readers (`GET /api/workers/{id}/frames`,
  `raw/sessions/index.jsonl`, and `cognition.md` tells the agent where its own stream is), so the
  substrate has a consumer. What is still undecided is when, if ever, to reach for the wire's own
  `thread/resume`; threads open `ephemeral: true` today, deliberately.

---

## Settled — do not re-file, do not rebuild

Each of these was removed or refused **on purpose**, most of them after being built and lived
with. Re-showing one costs a round trip and buys nothing.

| | |
|---|---|
| **The clock** | Removed 2026-08-09. There is no host scheduler and there will not be one. The host's timing surface is the loops that already exist — Cognition's glance-up, the reflection backoff, the check-in deadline — and everything past that the agent arranges with the shell it already has. A `due` is read and ordered, never fired. What that costs is stated once, in [`host.md#glancing-up`](arch/host.md) |
| **Presence-gated speaking** | Removed 2026-08-11, on the user's report from living with it. An open channel answers *is a window subscribed*, never *are you reading*, so the gate was quiet on someone sitting there and talkative to an empty desk. It was only ever load-bearing because words did not keep; the append-only conversation removes the loss and with it the reason to detect anything. Only `speaker_attached()` survives |
| **The host marks a hand-down as owed** | Withdrawn 2026-08-19. It could not know: the host knows *a* hand-down went out, but whether *this* message answers it is a reading of message against request, and only Reaction holds both. The code set the flag on **every** turn a person spoke into — "ok" handed down too — then spent it on whichever Cognition message arrived first, so a background finding could be announced as the awaited answer while the answer behind it rendered bare. At its best it bought one sentence in a prompt with nothing enforcing it: a check that was really guidance. The rule is now a section of `reaction.md`; `agents.md`'s "the host is what knows the difference" is gone |
| **Reflection is unaddressable** | Closed 2026-08-04. Nothing addresses it and nothing is meant to: no prompt names it as a recipient, and it wakes on its own backoff. "Unreachable" was measured against a general reachability rule it was never a client of |
| **The room / one-capture-at-a-time** | Cut 2026-08-12. Scenes are gone from the code, so this would have been *new* machinery dressed as restoring something. What is left of it is two mics at once, and a hard slot is the wrong answer — this codebase's answer to "who is speaking" is soft evidence the agent weighs, not a winner the host picks |
| **A name is leased** | Deleted 2026-08-13, the user's call. A lease loses you your name for going quiet, and every link and QR you handed out then points at a stranger. A name belongs to an account, permanently; the core credential went with the lease |
| **Subdomain addressing** | `hi-agent.xyz/ana`, settled. One certificate, no wildcard issuance, no per-handle DNS. It was changed to subdomains and reverted — recorded because the failure is reusable: it was raised as a choice to *re-open* and then taken on an adjacent "sounds good". Overturning a settled decision needs the decision |
| **Closed tasks accumulate** | Nothing prunes them. The invariant says never pruned *while open*; closed and cold ones age out like ambient identity clusters |
| **Ping-pong is possible** | Two long-lived agents can message each other indefinitely. Guided by prompt, logged rather than blocked |
| **A worker's `Bash` can read the auth token from its own env** | Non-hacker threat model |
| **No hand-edit lever** on prompts | Load-bearing — see [`arch.md#character`](arch/arch.md). There is no `*.local.md` override layer either; that was deleted, not deferred |
| **Cross-scene ambient awareness is weaker** than the old global digest | Continuity routes through Cognition instead |

**Retired vocabulary — do not reintroduce.** Verified absent from `src/`:
arbiter · delegate · ask · surface · handoff · notify · spawn · see · alarm · WorkerId ·
ToolCallStub · FollowMailbox · `Address` (any form) · scene-as-address · Deliberation ·
`WORKER_SYSTEM_PROMPT` · `any_host` · the ACP session vocabulary.

Each was retired **by deletion**, never by deprecation — a compatibility path kept "just until
the replacement lands" is the thing that quietly becomes permanent.

---

## Where each layer stands

**`FOUNDATION`** — most built, and the messaging layer is not only closed but visible: every
agent-to-agent crossing is recorded in both directions and the event log has a reader. The
session stream is kept verbatim per session under a run **and now has consumers**. The renderer
is proven against a real browser and has a caller (`review_view`).

**`DATA`** — log, episodes, facets, skills, views, forgetting, and both ends of the task ledger.
The stores were never the problem. Cognition is sole writer, with a test pinning that exactly one
prompt carries the instruction.

**`CORE`** — least built, and **on purpose**: the clock and the presence gate were not deferred
but removed, so the loop paces itself inline and that is the finished shape. What is genuinely
unfinished is behaviour, not structure — the vendor gate is half-classified, and `record_reflex`
can never fire.

**`AGENTS`** — three rungs plus workers, all real and wearing their own clothes. Every worker
kind is a prompt file; the prompts no longer name tools nobody holds. Reflection is a standing
rung with its own worker host.

**`SURFACES`** — who may reach the core is decided: two acceptors, one credential in two
presentations, the loopback path unchanged, and the relayed shape deployed. Text and audio solid
end to end. Vision journals and wakes; a handed file arrives with its locator. What is absent is
push.
