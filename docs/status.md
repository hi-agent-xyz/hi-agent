# Status — what has been watched running

Code is the source of truth for what exists. [`docs/arch/`](arch/) is the goal state. `git log`
holds how it got here, in full. **This file holds the one fact none of them can: whether a
mechanism has ever been observed running against a live instance.**

That is the distinction this repo keeps paying for: **built is not watched.** Nearly everything
here typechecks and passes tests. A dead soul seed, a write-only verb, a switchboard with no
readers and a frame log that kept nothing all shipped green.

So this file explains nothing. No mechanism, no rationale, no measurement, no history — those are
code, `docs/arch/` and `git log` respectively, and a copy of any of them here is a second source of
truth that goes wrong silently. An entry is a name, a pointer, and either the date it was watched
or the run that would settle it. If a line here starts describing *how* something works, delete it.

Three states:

- **watched** — observed against a running instance from ground truth *outside* the conversation
  (`server.log`, `GET /api/sessions`, the frame log under `memory/raw/sessions/`, disk artifacts) —
  never from what the agent said it did
- **built, never watched** — green tests, no live run
- **no code** — named in a prompt or a design, absent from `src/`

Method for moving a row up: [`CLAUDE.md`](../CLAUDE.md) § *Testing user journeys live* — terse
boss, don't lead the witness, verify every claim.

The architecture refactor's own log — every rung, every reversed decision, and the reasoning that
cost a round of being wrong first — is `git show 0294cde:arch-refactor.md`. The narrative this file
carried until 2026-08-28 is in `git log -- docs/status.md`.

---

## Built, never watched

Green and unexercised. Ordered by what breaks worst if wrong.

| | The run that settles it |
|---|---|
| **An agent-written value that steers code is parsed** — `consolidation_cursor`, `after_cursor`, `heartbeat::consolidate` returning `Pass::Swept`/`Pass::Skipped`, `reflection::note_pass` | Watched failing 2026-08-26, at a cost of thirty-five hours of memory: a hand-written episode id froze the cursor and reflection ran zero passes across a thirty-three-hour process. **Nothing has watched a single real consolidation pass through the fixed code.** Confirm `reflection fired` within a cadence of boot and a new directory under `memory/episodes/`; then plant an episode carrying a slug `to_id` and confirm the `warn` names the file, the cursor does not move, and passes keep firing. The backstop cannot currently fire — `note_pass`'s `warn` has never appeared in a log, and `STALL_WARN = 3` is unproven against a real cadence. Watch the first boot drain the frontier, 150 signals at a time |
| **Cognition's brief has a writer at last, and it is Reflection** — `reflection_prompt` interpolating `{cognition_memory}` into `prompts/seed/cognition.md` | **The file has never existed.** `agent_window` has read it for as long as it has existed and nothing ever wrote it, so Cognition's carried-forward block has been the empty string on every turn of every run. Nothing has watched Reflection write one. Over a few settling passes, confirm the file appears and read it against its own test — the failure mode is a *second copy of the ledger*, which is already projected into Cognition every turn. Then the real one, which needs a restart: get work genuinely mid-flight, restart, and read Cognition's first turn |
| **A name is not a key** — `work_record` folding case and `-`/`_`/space on `systems:`, an ambiguous fold counting as a miss, a miss answering with the roster | Watched failing 2026-08-27: a task carrying `systems: wechat` printed *"No record of this system yet"* in the brief's most-read position while `systems/wecom/facet.md` held the whole procedure. **No live brief has rendered through the fix.** Re-run that case and read the worker's first minutes in the frame log — it should open the right record or say plainly it has no precedent. The over-correction to watch is the model making the guess the code refused: a worker picking the nearest name off the roster. If that shows up, cut the roster, not the fold |
| **A working session's steps, readable by its owner** — `registry::steps`, `session_messages_report` | Hand out an errand that runs minutes without writing files early; call `hi_session_messages` mid-run and confirm ordered steps with ages that move. Underneath it: *does an owner that can see its worker still take the work back?* Re-run the cancel-vs-duration split against a later store |
| **Every review surface re-reads itself** — `useLive` in `@hi/core` | Nothing has watched a tick fire. Three parts, one instance: pair a phone with `reach` open and the card must appear without a click; drag a `tasks` card past 8s and the board must not move under the hand; leave `stats` open and confirm no `/api/stats` in `server.log` until focus returns |
| **Expression is two gates** — prose across four prompts, zero code | Re-derive from `data/memory/raw/text/*/text.jsonl` after a week and compare to the 2026-08-20→26 baseline in `git log`. Watch for over-correction: terse where an answer was owed; a job id softened into prose; `person-reader` writing what someone *cannot* follow |
| **The screen answers to the conversation** — `ViewBus::shown()`, `reaction.md` | Two live-conversation parts. *Backwards*: get a view up, talk elsewhere for turns, return to the subject in passing — does the view come back, and did the ref come from the new block or from further up the session? *Forwards*: have a worker finish something unrelated mid-thread. Watch for views going up merely *because the agent can see the list* |
| **The frame is the window** — `.hi-view-fill`, `--hi-safe-top` / `--hi-chrome-bottom` | No live face has shown it. Put up a view with an `inset:0` cover image and confirm it reaches all four edges; judge whether the light-skin scrim reads as controls or as coins; bring up a few pre-change `data/views/*` and see whether a headline lands under the traffic lights |
| **A view goes up on its first clean render** — `STAGE`, `hi_review_view` | Open the face on phone and desktop, resize, and confirm `hi_review_view` renders the frame of whichever spoke last. Time the gap between first clean review and the view appearing — seconds, not the rest of the build. Then watch a refined version replace the first *under someone mid-read* |
| **One `Message` for the whole conversation** — `JournalEntry` splitting four ways | One boot against a *copy* of a real `data/`: every message still there, in order, same face, none gaining a face it lacked. Then type a line, hand a file, speak one — one `message` line per act, the file's name in its own field, and no `⟨…⟩` in any body. `Appraisal` has a kind, a home and no producer |
| **A resumed rung is not re-seeded** — `warm_reaction_session`, `AgentSession::resumed()` | Read the *first real turn* after a resume and judge whether it is as oriented off a cold memo as it was off the seed |
| **Reopening what the host ended** — [`agents.md#across-a-restart`](arch/agents.md) | One restart with one worker mid-turn and one idle-but-open: expect `reopening=2`, both keeping their slugs, the mid-turn one checking what its last steps did rather than redoing them, the idle one taking no turn. Then delete a rollout from `data/codex-home/sessions/` and confirm it does not come back and its owner is told. Then the mail half: a message sent mid-turn must be re-posted behind the `(restart)` gap line |
| **A promise the conversation carried reaches cognition** — [`reflection.md`](../src/identity/reflection.md) | Take work on in conversation, let a worker get partway, restart before any row exists; confirm reflection's settling pass sends `cognition` the promise and a row stands after the next glance-up |
| **The switchboard's traffic ring** — `registry::mail`, `GET /api/workers/mail` | Driven end to end, but the sends were a test's. Unwatched under a *working* agent: whether 300 messages is a useful window or an afternoon evicts the morning, and whether a worker's report runs past the 4k clip |
| **The floor** — the mouth refusing a turn whose room isn't the voice's | A real partial stream; curl cannot drive one |
| **The check-in** — `say(text, back_in)` | One errand over 5 minutes: hand it over, **don't ask**, watch for `check-in fired` in `server.log`, and judge whether what is spoken is progress or "still working on it" |
| **The duty inbox's edges** | Plant a `serving` facet with a `start_key`, POST a burst, idle past the TTL, POST again; then restart mid-burst and confirm the glance-up still finds the unhandled rows |
| **A genuinely new machine keeping its name** | `device_id` is machine-derived, so two data dirs on one Mac share an account and a live run on one box cannot tell the cases apart |
| **The Windows build** — `make exe` / `make installer` | Cross-compiles and has never been run on Windows. `write_browser_shim`'s `#[cfg(windows)]` arm mirrors the POSIX logic on paper and nothing has executed it |
| **The Docker shape's gate** | A published port is off-box, so an existing deployment is gated from first run. Reasoned about, never exercised |
| **Nothing opens during the drain** — `AgentLayer::session` refusing after the shutdown signal | Quit with rungs live, confirm the last `thread` row per rung in `memory/raw/sessions/index.jsonl` is still the run's own, and that the next boot logs `resuming=2` on those ids |
| **A session swap that fails or times out** | Both arms written, neither provoked |
| **The conversation surviving every view** | Pull the vendor credential, watch the outage notice come up, confirm the conversation is still there and still typeable |
| **A show taking the window with it** | Park on an older card, have something shown, confirm the stage moves; re-show the same destination and confirm it still moves; a dismiss must move nobody |
| **The Sessions tree** — `forest` / `layout` / `fitted`, `d3.tree()` | Mounted against a live roster, so it draws. What no reading covers is **churn**: watch a worker start under Cognition and confirm it animates rather than jumps, and that a too-wide tree scrolls with its edge fades rather than hiding a rung |
| **The views band opening on the bookmarks row** | Open the band while parked on a `factory/` surface far along the row and confirm its chip is on screen |
| **Working ahead** — journey [34](user-journeys/34-a-step-ahead.md) | Hand over something whose next step is outward, then say nothing, and confirm it stopped at the door. Prompt-level throughout; nothing pins that the agent acts on it |
| **The seam between the rungs** — `reaction.md`, `cognition.md` | Ask about something whose short name is ambiguous in the ledger, then read the hand-down in the frame log: their word must be in it, quoted, any binding marked a guess. Then hand over builder/reviewer work and count what reaches the conversation — the thing landing, not one line about the gates |
| **A worker names the task it serves** — `hi_create_worker` refusals, `tasks::named` | Ask for something the ledger has never heard of and read the frames: the model must name a row from the list it was handed or write the facet and retry. If it thrashes or names the nearest row, the fence costs more than the label did. Also open: whether 30 offered rows is right in a store with 108 subjects |
| **One promise, one row** — `task-manager.md` § *One promise, one row* | Managers run (69 dispatches in one install), but folding is new prose none of them has carried, so the first fold is still ahead. Watch one fold two rows that are one job and *decline* two that merely share a subject, and read whether the cancelled row says where the promise went |
| **The `ahead` count** — `hi_create_worker(ahead)` → `N/M errands started ahead` | Self-reported, so it can only undercount, and a zero is two findings wearing one number. Read it against the wire frames' actual `hi_create_worker` arguments on a run where working ahead plainly happened |
| **The inbound-text privacy boundary, in its new shape** — `POST /api/in/text` filing, `AgentSession::prompt` substituting | Nothing in the new shape has run. Re-run the 2026-08-19 journey: paste a key, confirm the conversation still shows it and the log still holds it, then ask for it to be used against a real endpoint. Unwatched too: a rung opening a ref by path since the placeholders came back, including the `keep/` fallback for a faded original |
| **`tasks::reconcile` catching up the tail it used to abort on** | Nothing has observed a brain turn reconcile the 61 records that sat behind the deleted refusal |
| **A waiting row stays `doing`** — the widened sentence in `task-manager.md` | Whether managers record the wait and leave the row in `doing`, and whether the relay in `cognition.md` (a close or a new block is news, said in the turn the report lands) survives a busy turn. The seven wrongly-closed rows are still `done` on the live instance; nothing reopens them |
| **The timeline vocabulary** — `created` / `delivered` / `waiting` / `update`, `waitsOnPerson` | No mind has written a line in the new vocabulary. The *Needs you* block, the card marker, the greyed superseded wait and the lifecycle verbs are unit-tested and never rendered in a browser |
| **A `waiting` line naming where the person acts** | No mind has written an address into one. The live KT8-059 row carries none; nothing backfills |
| **The acceptance line at open** — `created` carrying what would make this right | Counted 2026-08-25: 0 of the open rows had one. Deliberately no code check — after open there is no valid response to its absence. Count again against a later store |
| **The board's extra fields** — `statusSince`, `systems:` / `report_to:`, the `onIt` roster join in `views/factory/tasks.jsx` | No live instance has drawn them. Settles: whether promoting `systems` to tags is right for records whose value is not a list of systems, and whether *Nobody on it* across most of the `doing` column reads as an alarm or as wallpaper |
| **Reflection deciding what to build** — step 3 of the settling pass | The ranking half is watched; the deciding half needs reflection to fire on a real backlog, and it has not since the change. What to look for is a negative: that it *doesn't* propose tools on thin evidence |

---

## Watched

Observed against running processes, dated where the date is known.

- **The spine** — boot with both sceneless rungs registering synchronously · `say` adopted from the
  first turn · `show` and the first-meeting welcome · the character actually read · the frame log
  filling under a run id · Cognition woken by a hand-up and hosting its own workers · the task
  ledger written · Reflection firing on its own backoff · the session swap on all three long-lived
  rungs · Cognition resuming a standing duty after a host restart · the duty inbox
  (`POST /api/in/duty/<start_key>`) taking a burst into one handler.
- **Topology T0–T3d, through the deployed CDN** — the local shape · the directly-public acceptor ·
  the relayed shape end to end through the deployed community and Tencent EdgeOne · the gate in
  both credential presentations, with CSRF · all three pairing paths, the device list and
  revocation · the app's roster and local proxy across two cores · a name claimed by a signed-in
  account, permanent, refused to a second account and to an anonymous one, surviving a wiped data
  directory, and released back · the subpath prefix, emitting its own base and serving through the
  tunnel · `_builtin/reach` · the roster at the app's own `/app` · the relayed page rendered in a
  real browser, React mounting and SSE reconnecting.
- **The cache-control rule, against the real edge** — `topology.md` § *Nothing behind the gate is
  `public`*, enforced at the core and pinned by a test.
- **A large paste is a handed artifact, not an utterance** — 2026-08-24, one boot.
- **A tool is found and run** — 2026-08-26, isolated instance: `bin/browser` resolved off the
  session PATH, `browser.md` opened before anything ran, `curl` correctly preferred on a static
  page and `browser --dump-dom` reached for when `curl` returned a shell.
- **The workshop's execution layer nests** — 2026-08-27: `bin/factory/` rewritten every boot, a
  hand-written `bin/browser` winning over it, the child PATH ordered as specified.
- **`hi mcp` reaches a real MCP server, both transports** — 2026-08-27, against
  `@modelcontextprotocol/server-everything`.
- **Commands counted by the program they ran** — 2026-08-27, a live errand's `sed` in
  `commands_by_name` with its failure counted.
- **The hot level inside a live session's `baseInstructions`** — 2026-08-27, three seeded tools with
  their purpose lines and `equipping-a-tool` degrading to a bare name.
- **Tool ranking by recent usage** — 2026-08-27, score-descending with mtime breaking only ties.
- **`SKILL.md` in a directory read as a note** — 2026-08-27, re-run on the same errand: `skills/`
  came back 52 K instead of 76 MB, identity and excerpt right.
- **`tasks::reconcile`** — 2026-08-19 dry run, 58 records rewritten on the first pass and 0 on the
  second; 2026-08-24 against a copy of the live 101-record store, 0 rewritten and no prose lost.
- **The running record's schema and panel** — 2026-08-24, release binary against a seeded store in a
  real browser, including a live rung writing an `update` line unprompted.
- **A URL in a waiting line is clickable** — 2026-08-26, release binary rendered through
  `GET /render/view` in real Chromium and driven over CDP, both skins.
- **`.history/` keeps a file a second writer replaced** — re-ran the original loss: a whole-file
  overwrite from outside the process, and the destroyed version still on disk and readable.
- **The account above the timeline** — 2026-08-26, headless against copies of two real records.
- **`every_review_surface_renders_on_a_core_that_has_nothing_yet`** — all nine factory views through
  the server's own pipeline in real Chromium against empty endpoints. It answers *does it draw*,
  not *does it draw right*.

---

## No code

Named in a prompt or a design, absent from `src/`.

- **Nothing notices when no Task Manager has run.** *Not* a reachability gap — dispatch is fully
  wired: `TaskManager` is in `WorkerType::ALL` so it reaches the tool schema's `type` enum, both
  standing rungs hold `hi_create_worker`, `registry` gives the kind its no-subject exemption, and
  `cognition.md` § *Handing the ledger down* tells Cognition in as many words that starting one is
  its job. Managers do get dispatched — 69 of them in one install's frame logs. What is absent is
  the **observation**: nothing under `src/mind/` or `src/body/` reads `TaskManager` at all, so
  nothing records that a manager ran and nothing notices when none has. `tasks::projection` already
  counts rows past `past_idle_boundary`; it cannot add *"and no manager has run since X"* because
  that fact is not kept anywhere. The chain is *pulse fires* (code) → *Cognition starts a manager*
  (prompt) → *manager files* (prompt), and only the first link is code.
- **A recently-closed `serving` row keeping its place in the manager's window** — closed when,
  carrying a `verify:`, not checked since. The store surfaces staleness; the manager does the
  looking, because every live `verify:` is prose no code can run. This is what would have caught
  `feishu-it-group-watcher`.
- **The reconcile pass reporting what it cannot fix** — misfiled `verify:`, unknown status words, a
  task directory with no row, a closed record with no instant — as facts in the manager's window,
  never as refusals.
- **The Tool Manager** — the only designed route outside the hot set. Deliberately unbuilt: the
  workshop fits in the window, so `grep -rEn "^(purpose|description):"` *is* the lookup.
  **Reopens when** `hot_inventory` hits its byte cap on a real install.
- **post** — the push service, and with it waking a surface.
- **Mail for a sleeping core**, and therefore core-to-core addressing. An inbound request is
  answered `asleep`; nothing queues. Also the trigger that makes keypairs non-optional.
- **Refusing to route for a surface reported lost** — the revocation case a sleeping core cannot
  serve.
- **Credentials in the OS keychain on the desktop.** iOS has `KeychainStore`; the desktop app keeps
  them in its config store.
- **A core on iOS.** Blocked by the wire being a spawned binary. (`app/apple/ios` is a client.)
- **The retention question.** `data.md#keys-passwords-and-the-one-question` describes a one-time
  *this / all / none* choice; ingest files every detected secret automatically.
  `identity::soul_tests::prompts_are_honest_about_current_auto_retention` pins the prompts to saying so.
- **A shelf for prepared things.** Working ahead has no store; nothing carries a prepared artifact
  across a wake or a restart. **Reopens when** a run redoes or forgets prepared work.

---

## Open forks

- **`use:` cannot be obtained by asking.** Two attempts at getting the key in prose, neither took;
  the model reliably writes `purpose`/`description` and a `## Run` section. Dropping `use:` to one
  key reverses a decision taken 2026-08-26, so it is not being taken quietly.
- **Barge-in stop path** — core-owned or forever client-side? Today it is in the browser and
  `Floor::mark_flush` ([`floor.rs:497`](../src/body/reaction/floor.rs)) has no caller outside its own tests.
- **`record_reflex` is declared to no role** — a reflex can be fired but never written. Give it a
  live role or delete the module.
- **Per-install prompt instances** — should `data/prompts/` ship knowing whose agent it is?
  Currently no: it learns by meeting them.
- **Replay of a finished session's frames.** The frame log has live readers; nothing reads a
  finished session back.

---

## Deliberately absent — do not re-file, do not rebuild

Each was removed or refused on purpose, most after being built and lived with. The reasoning is in
`git log` and in [`docs/arch/`](arch/); this list exists only so it is not re-proposed.

| | |
|---|---|
| **The clock** | Removed 2026-08-09. No host scheduler, ever. A `due` is read and ordered, never fired; the cost is stated once in [`host.md#glancing-up`](arch/host.md) |
| **Presence-gated speaking** | Removed 2026-08-11. Only `speaker_attached()` survives |
| **The host marking a hand-down as owed** | Withdrawn 2026-08-19. Only Reaction holds both sides of the reading. The rule is a section of `reaction.md` |
| **Reflection being addressable** | Closed 2026-08-04. It wakes on its own backoff |
| **The room / one-capture-at-a-time** | Cut 2026-08-12. Two mics at once, weighed as soft evidence, not a slot the host arbitrates |
| **A name is leased** | Deleted 2026-08-13. A name belongs to an account, permanently |
| **Subdomain addressing** | Settled: `hi-agent.xyz/ana`. Changed to subdomains once and reverted — overturning a settled decision needs the decision, not an adjacent "sounds good" |
| **An `onIt` field on `GET /api/tasks`** | Cut 2026-08-26 and re-landed as a view-side join against `GET /api/workers`. No DTO, no server staleness rules |
| **A `blocked_on()` classifier** | Cut 2026-08-26, same day it was built. Code should do only clear and decisive logic; that was 135 lines deciding what a `blocked` line means |
| **A dispatch that opens the missing task row** | Cut. A ledger that fills itself is a worse list than a missing label |
| **A gate on collision-prone facet writes** | None added, none planned. A guard on a field an agent must remember to fill is silently absent when it forgets |
| **Filtering outside the two seams** | Tool results, the system prompt, mail, exported diagnostics and codex's own shell are unfiltered by scope decision — see [`privacy.md`](arch/privacy.md). Nothing non-text is detected |
| **Closed tasks accumulating** | Nothing prunes them. Never pruned *while open*; closed and cold ones age out like ambient identity clusters |
| **Ping-pong between two long-lived agents** | Possible, guided by prompt, logged rather than blocked |
| **A worker's `Bash` reading the auth token from its own env** | Non-hacker threat model |
| **A hand-edit lever on prompts** | Load-bearing — [`arch.md#character`](arch/arch.md). No `*.local.md` override layer either; deleted, not deferred |

**Retired vocabulary — do not reintroduce.** Verified absent from `src/`: arbiter · delegate · ask ·
surface · handoff · notify · spawn · see · alarm · WorkerId · ToolCallStub · FollowMailbox ·
`Address` (any form) · scene-as-address · Deliberation · `WORKER_SYSTEM_PROMPT` · `any_host` · the
ACP session vocabulary.

Each was retired **by deletion**, never by deprecation — a compatibility path kept "just until the
replacement lands" is the thing that quietly becomes permanent.
