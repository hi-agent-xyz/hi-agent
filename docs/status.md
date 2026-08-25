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

**The running record got a shape, a store-written spine and a surface, 2026-08-24.** The
2026-08-23 version was guidance only: three writers were told which parts of one prose body
were theirs, nothing rendered differently, and nothing was ever watched writing them. It is
now a schema. The body carries a `## Timeline` heading and, under it, dated append-only
lines of five kinds — `asked` (Cognition, at open), `landed` / `blocked` / `checked` (the
worker and the manager), and `moved`. Prose above the heading passes through untouched.

**`moved` is written by the store**, in `Task::stamp_transition`, on the same
`tasks::reconcile` pass that already stamps `status_since` — so the spine of every task's
history exists whether or not an agent cooperates, and the two writers of a transition
(code, which is told; the pass, which finds out) are kept from both claiming the same one
by `write_task` updating `LAST_SEEN`. Nothing is backfilled: a record predating the pass
gets a history starting at its next transition.

The panel was rebuilt around it (`views/factory/tasks.jsx`): `asked` pinned at the top,
the record newest-first, the long prose folded behind a disclosure, and the latest line on
each card. That last part is why this was worth doing — median live body is 3.2 KB and the
largest is 48 KB, all of it previously dumped into one `<div>`.

**Watched, 2026-08-24, against a copy of the live 101-record store**: the first pass
rewrote **0** records (the store is already canonical, so the schema costs nothing to
adopt), no record lost a character of prose, no status moved, and no record grew a history
it did not earn. A `todo → doing` edit made by hand then produced exactly one line —
`- 2026-08-24T08:35:21Z moved — todo → doing`, appended below untouched prose — and the two
passes after it wrote nothing. No live body carries a clashing `## Timeline` heading.

**Also watched, 2026-08-24, and it found the one defect.** A release binary was run against
a seeded three-task store and the board rendered in a real browser: `asked` pinned, the
record newest-first with per-kind rails, the prose folded, and each card carrying its
latest line. While it ran, a live rung read the ledger and **wrote a `checked` line into
the running record unprompted, in the format the prompt specifies** — the half that had
never been watched. The store wrote its `moved` line on the same turn.

**And in the same turn it wrote `status: blocked`.** `blocked` is a *record kind*; it is
not one of the five status words, so `TaskStatus::parse` fell back to `Todo` and a task
that was underway and stuck came back reading *not started*. `is_malformed` caught it and
the panel said so in red, which is the designed behaviour working — but the collision was
introduced by this change, so the fix went where the mistake was made: all three prompts
now say a kind is not a status and that a blocked task stays `doing`. **The re-test is
watching a rung meet that wording** — nothing has yet.

**Built, not watched:** `asked`, `landed` and `blocked` lines written by a mind (only
`checked` has been seen), and a Task Manager appending its closing `checked` line before
it moves the status word.

**And the shared-folder collision is answered in prose, on purpose.** Two sessions wrote the
same path under one task; the loser's briefing was replaced whole and the winner's file was then
read by the loser as its own, with no error and no copy. The scan that found it: 1,635 facet
writes over 578 paths, 95 written by more than one session, 148 whole-file shell overwrites,
two confirmed silent losses in a week — a **floor**, since it can only resolve literal paths and
cannot see through 326 `write_text(…)` calls or 575 `tee`s. `facets.rs`'s *last-writer-wins, and
that is fine* was correct about `facet.md` and silently general about its siblings; that comment
is now scoped, and `general.md` carries the write-verb rule (`apply_patch` checks, a heredoc
does not). No gate was added and none is planned — the rejected `deliverable: <ref>` is the
precedent: a guard on a field an agent must remember to fill is *silently* absent when it
forgets, which reads exactly like a clean delivery.

**The keeping half landed 2026-08-24, and is watched.** Guidance moves the write verb and
cannot keep the bytes, so `mind/memory/task_history.rs` keeps them: `reconcile` — the pass
that already re-reads every record on every window build — now copies each file in a task
folder into `<subject>/.history/` before anything can rewrite it. It rides on that pass for
the pass's own stated reason, that *a pass that re-reads the bytes cannot be walked around,
because it reads whatever is actually there however it got there*. Content-addressed, so an
edit-and-undo keeps two versions and not three; `(len, mtime)` pre-checked, so an untouched
store costs one `stat` per file and no reads at all. **No size threshold**, deliberately: a
cap that silently skipped the largest file would drop history for exactly the artifact most
expensive to lose and look identical to having kept it, and every task folder in every store
on hand holds one `facet.md`, the largest 6 KB.

**Watched, by re-running the original failure.** A release binary against a seeded task
folder: pass one kept `briefing.md` and `facet.md`; a `cat > briefing.md <<EOF` from outside
the process replaced the file whole, exactly as the real loss did; the next pass kept the
replacement, and the destroyed version was still on disk and readable. Five passes over a
task the agent was actively working produced six entries and six distinct contents — no
duplicates, nothing kept twice, 24 KB.

**Three things it does not do**, none of them oversights. It does not **prevent** a
collision — a lock or a refused write is a gate, and both writers still land. It does not
**detect** one: the pass sees that a file changed, never who changed it, and a body changing
without a status moving is ordinary work rather than a signal, so nothing goes in the window
and nothing should until the writer's identity is available. And a version written *and*
replaced between two passes was never observed, so the exposure window is one brain turn
rather than zero — it is not zero, and no cheap mechanism makes it zero. Only files directly
in a subject directory are covered; a nested deliverable is not.

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

**A large paste is a handed artifact, not an utterance — watched 2026-08-24, one boot.** A
61,890-byte body stayed words (`Content::Text`, text channel, 61,890 chars in the journal); a
334,890-byte body arrived as `Content::File` on the file channel with `bytes: 334890`, a peek,
and a blob that `cmp` reports byte-identical to what was posted. The line the mind was handed
was `>/file handed you a file: pasted-….txt (text/plain; charset=utf-8, 327 KB) ⟨ref: …⟩`
followed by `┆`-marked opening rows — **and 327 KB of log never entered the prompt**. Off that
line alone the agent said *"I've got the log file you sent"*; asked afterwards how many distinct
workers appeared in it, it opened the ref and answered *"Seven distinct workers: worker-0
through worker-6"*, which is correct. `POST /api/in/text` streams and carries no size limit at
all now — it inherited axum's 2 MB default before, and answered a bigger paste with a 413 the
face discarded silently.

What that boot did **not** reach: a body large enough to matter as a *stream* (the biggest was
327 KB, so nothing has exercised writing through for minutes), the `SCAN_MAX` ceiling where the
credential scan starts reporting `partial`, and the composer's put-the-draft-back path — the web
suite has no DOM harness, so that one is read-only-verified. Two artifacts pasted inside the
same second still overwrite each other; `media_rel_path` has one-second granularity and no
uniquifier, which dragged files have always shared.

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
| **A view goes up on its first clean render, and a review renders the surface someone is on** — `STAGE` is a list of surfaces with the most recent reporter at the head, every face reports under its own id (the popover excepted, and it now says so with `?chrome=popover`), and the builder's bar is split into a machine-checkable ship gate and a refine pass that runs with the view already up ([`stage.md` § *The frame is a surface…*](arch/stage.md)) | Green — 972 lib tests, the store's own semantics pinned in `view_render`, the two prompt halves pinned in `identity` — and **not one line of it has run against a face.** The measurement it was built from, over 136 worker sessions 15–24 Aug: 36 minutes median from a builder starting to the view reaching the screen, of which the headless browser is 2.4% of builder active time and the review loop 29%; 41% of builder review calls rendered a frame under 500px that no surface ever reported, and those renders sent the builder back into the source 28% of the time against 13% for a real one. **The re-test, in three parts.** *The surface*: open the face on the phone and on the desktop, resize the desktop window, and confirm `hi_review_view` renders the frame of whichever spoke last — then open the popover and confirm it moved nothing. *The gate*: ask for a view and time the gap between the builder's first clean `hi_review_view` and the view appearing; it must be seconds, not the rest of the build. *The pass*: watch what happens after — a refined version has to reach the screen over the first one, and the thing nobody has seen is whether a replace under someone mid-read is tolerable or infuriating, which is the whole risk the split takes on. What no test can reach: whether builders actually stop inventing widths, since the prompt is the only thing that says so |
| **One `Message` for the whole conversation** — a typed line, a spoken one, a handed file and one `say` are a single value minted at the boundary and handed unchanged to the journal, to Reaction and to the conversation; `JournalEntry` splits four ways (`Message`, `Presentation`, `Observation`, `Internal`) and lines written as `signal_in`/`signal_out` classify on the way in, with nothing on disk rewritten ([`message.md`](arch/message.md)) | 935 lib tests and every integration binary are green and **no live instance has taken one line through the new path**. The durable half is what to watch, because the legacy classifier is exercised only by fixtures *written alongside it* — and all three bugs found while building it lived exactly there: the filename parse ran straight over the `⟨ref:⟩` sitting behind it, only the **first** `⟨…⟩` marker was ever read so a voice tag positioned after a ref went unseen, and `⟨voice: 老王 ~0.82⟩` wrote the similarity score into `subject`, opening a facet named `老王 ~0.82` that no later match ever joins. Each was caught by a test asserting the old contract, not by reading the code. **The re-test, in one boot**: point a fresh instance at a *copy of a real `data/`* and read the conversation back — every message that was there must still be there, in order, with the same face beside it and no message gained a face it did not have. Then the live half: type a line, hand a file, speak one, and confirm the journal holds one `message` line per act, the file's name in its own field, and **no `⟨…⟩` in any body** — carriers set fields now, and the markers are written only into the prompt. `Appraisal` has a kind, a home and **no producer**: relevance is the thing this refactor existed to make possible and nothing computes one yet |
| **A resumed rung is not re-seeded** — `warm_reaction_session` skips `seed_session` when the thread came back, and `AgentSession::resumed()` is how it knows | The seed opened *"Nobody has spoken yet"* and re-handed 21k chars of transcript tail, ledger and roster to a thread that had been continuous across four runs and held all of it — measured on the 2026-08-21 boot, where `resumed the previous run's thread role=reaction` at 08:31:38.160 was followed by `seeding the thread seed_chars=21324` 19ms later. Now it logs `reaction resumed its thread; no seed`. Unwatched: whether the **first real turn** is as well-oriented off a cold memo as it was off the seed — the window is re-projected into it either way, so the claim is that nothing is lost, and nobody has read that first turn |
| **Reopening what the host ended** — every worker its owner had not closed comes back on its own thread, under its own slug, with its unread mail; mid-turn ones are handed a `(restart)` note telling them to establish what actually landed, ones that were merely waiting are handed **nothing** and go back to waiting ([`agents.md#across-a-restart`](arch/agents.md)) | Replaces the *offer*, which only fired on a crash: a clean quit recorded every live worker `Closed` and the next boot offered nothing. Measured before the change — the 2026-08-21T08:30:45 quit closed 16 live workers, 4 mid-turn (a KUT deploy among them), and the next boot logged `offering=0`. Unit tests reach the `by_host`/`interrupted`/`held` marks, the filter, the mail fold, the two ledger lines and the note's wording; **nothing has watched an errand come back.** The re-test, in one restart: with one worker mid-turn and one idle-but-open, quit and restart. Expect `reopening=2` plus `exchange=` and `owed=` on the boot line, `errand reopened on its own thread` twice with `handed=a turn` and `handed=nothing`, both keeping their old slugs. Then read the mid-turn one's first turn — it must go and check what its last steps did, not redo them — and confirm the idle one took **no turn at all**. Then the failure half: quit mid-turn, delete that thread's rollout from `data/codex-home/sessions/`, restart, and confirm it does not come back, its owner is posted why, and its task line says `could not be reopened`. And the mail half: send a worker a message while it is mid-turn, quit before it reads it, restart, and confirm the message is re-posted behind the `(restart)` gap line rather than dropped |
| **The switchboard's traffic ring** — every delivered `Registry::send` kept in memory (`registry::mail`, 300 messages, 4k chars each) and read back per pair by `GET /api/workers/mail?a=&b=`, which is what an arrow on the Sessions page opens | Real, and exercised only from a test's own registry: the endpoint was driven end to end against a live server (both directions, a third session's mail correctly absent) but the sends were the test's, not an agent's. What no run has covered is the ring under a **working** agent — whether 300 is a useful window or an afternoon's traffic evicts the morning's, and whether a worker's report routinely runs past the 4k clip. Nothing refused is recorded and nothing host-posted is (`Registry::post` has no sender, so no arrow could show it) — both deliberate, both unverified against what a reader actually goes looking for |
| **The floor** — the mouth refuses a turn whose room isn't the voice's to take (their voice sounding; a line went unheard), with a three-refusal backstop | A real partial stream. Both conditions are unit-tested; curl cannot drive a partial |
| **The check-in** — `say(text, back_in)` arms a wake, with a floor under an open-ended silence | One errand that takes >5 minutes: hand it over, **don't ask**, watch for `check-in fired` in `server.log`, and judge whether what gets spoken is progress or "still working on it" |
| **The duty inbox's edges** — TTL re-derivation, and a restart mid-burst | Plant a `serving` facet with a `start_key`, POST a burst, idle past the TTL, POST again; then restart mid-burst and confirm the glance-up still finds the unhandled rows |
| **A genuinely new machine keeping its name** | Covered by `TestRegistryNameSurvivesANewMachine`, not by a live run — `device_id` is machine-derived, so two data dirs on one Mac share an account and the live run could not tell the cases apart |
| **The Docker shape's gate** | A published port is off-box, so an existing deployment is gated from first run. Reasoned about, not exercised |
| **Nothing opens during the drain** — `AgentLayer::session` refuses once the shutdown signal is triggered, so no rung can mint a thread while the host is winding down | The failure it fixes was watched on 2026-08-20: a cold reopen during the drain wrote a newer `thread` row for Cognition and Reaction, and the next boot resumed those 14-record shells instead of the 34.6 MB thread the run was actually spent on. The guard itself is unit-tested only. Quit with rungs live, then confirm the last `thread` row per rung in `memory/raw/sessions/index.jsonl` is still the run's own, and that the next boot logs `resuming=2` on those same two ids |
| **A session swap that fails or times out** | Both arms are written (keep the warm session; discard the unresponsive one) and neither has been provoked |
| **The conversation surviving every view** | `ViewTraits` and the `.geom.json` sidecar are deleted, so `stage()` no longer has a `hidden` case at all — the popover/pill pair is the whole of it. Covered by unit tests on both sides; what none of them reaches is the outage, which is the one view that ever claimed the words. Pull the vendor credential, watch the notice come up, and confirm the conversation is still there and still typeable |
| **A show taking the window with it** | The parked window follows any show now, including a re-show of the same destination (the cursor drops on the newest history entry changing, not on its destination changing), and the server clears its `attention` on any show to match. Unit tests reach the server half only: park on an older card, have something shown, and confirm the stage moves; then re-show the same view and confirm it still moves. A dismiss deliberately moves nobody |
| **The Sessions tree** — the roster drawn as the ownership tree it is, packed by `d3.tree()` (Reingold–Tilford as Buchheim refined it) against one **constant** card height every card is drawn to, with one rAF animator owning card positions, the arrows between them, and the fade of a session starting or ending | Checked hard, but never mounted. The pure half (`forest` + `layout` + `fitted`) was run directly against a fabricated roster — ladder order, one rank per depth, no two cards overlapping in a rank, a parent centred over its children, the synthetic root d3 needs never leaking into the drawing, an orphan and an ownership *cycle* each drawn exactly once, and narrowing that stops at the card-width floor. The published-library path was checked end to end rather than assumed: the view's bare `d3-hierarchy` specifier survives the server's own esbuild transform (`--loader=tsx`, no `--bundle`), and `make build` emits it in `dist/importmap.json` against a real 14KB chunk. The drawing was rendered in real Chrome at 980 / 1600 / 2200px in both skins, and the create/end transition sampled frame by frame off the page's own rAF (two cards cross-fade, the ended one unmounts at ~330ms, everything settles at full opacity). All of that was a standalone page carrying the real stylesheet and a stubbed `/api/workers`. **The mounted view has since been seen against a live roster** — three rungs over six workers, the arrows following the `owner` chain — which retires the "never mounted" half of this row. The 2026-08-21 card pass (one height for every card, one-line titles, a live stage a screenful tall, room under the plot for the card's own shadow) was rendered off a running instance's real `/api/workers` in headless Chrome, including the empty roster and a fifteen-card tree; that reading mounted the file directly rather than going through the server's own view pipeline, and the pass has not been seen in the app itself. `make test` still reaches none of this: a factory view has no test harness, and building one is a boundary decision nobody has taken. What no reading has covered is **churn** — watch a worker start under Cognition and confirm it animates rather than jumps, and confirm a tree too wide for the frame scrolls with its edge fades rather than hiding a rung.

**2026-08-23 — the card is a fixed size and the arrows are controls.** Four changes, and the first three were seen rendering: the card height is now the constant `NODE_H` instead of the tallest measured card (a wrapped `doing` line used to push every card and the rank under it down 17px on a poll and back on the next), cards are 272×206 rather than 236×whatever, the meta line is clamped run-on text in both cards rather than a wrapping row of chips, and a **link arrow** is drawn from Reaction across to Cognition — the traffic edge the page never had, since Reaction creates nothing and everything it hands on goes up by `hi_send_message`. Rendered in headless Chrome through the server's own view pipeline (`ViewCompiler` + `/render/view`, not a standalone page) against a real switchboard roster of three rungs and three workers; that render also caught a live bug — a `<button>` centres its content, so with a fixed height every pill in a rank sat at a different y until the card became a flex column. **What is built and not watched: the click.** Every arrow now opens `Channel`, the exchange between the two sessions it joins — the panel itself was rendered by forcing its state open, so the *panel* has been seen with real data, but no pointer has ever hit one of the invisible fat hit paths, and no reader has focused one with a keyboard |
| **The views band opening on the bookmarks row** | The upper row's scroll-to-*here* has been watched; the lower row's is new, and its arithmetic was only checked in a standalone page carrying the real stylesheet — not in the mounted band, where the chip appears a render after the first `listViews()` answers. Open the band while parked on a `factory/` surface far along the row and confirm its chip is on screen |
| **Working ahead** — the handover carries the questions it provokes, and the reversible half of the likely next step is handed out in the same turn ([`agents.md` § *Working ahead*](arch/agents.md)) | Journey [34](user-journeys/34-a-step-ahead.md), written to be run without leading the witness. **The one that decides it**: hand over something whose next step is outward, then say nothing, and confirm it stopped at the door. This is prompt-level throughout — the identity tests pin that the prose is present and that the permission never travels without its boundary; nothing pins that the agent acts on either |
| **The seam between the rungs** — a name in the person's words crosses down quoted, with Reaction's reading beside it rather than in place of it; the agent's own housekeeping (gate verdicts, retries, contrast ratios, which attempt this is) does not cross up at all; a commit answers *what was done* and never *what was asked*; a name matching exactly one open ledger row is survivorship, not disambiguation; an ask you simply answer opens no row; one `task-manager` serves the whole ledger, never one per row ([`agents.md` § *The hand-down*](arch/agents.md)) | All six are prose in `reaction.md` and `cognition.md`, written off one failure watched on 2026-08-24: a bare "056" went down already bound to a commit, the rung holding the ledger read only what that binding pointed at, and eleven minutes of confident, internally consistent answers described a child regression instead of the contract it was a child of — ten of the ledger's 103 rows carry that ticket number, and exactly one of the ten was open. Every step after the binding was correct work. Nothing pins that any of the six changes behaviour. **The re-test, in two parts**: ask about something whose short name is ambiguous in the ledger, then read the hand-down in the frame log — their word must be in it, quoted, and any binding marked as a guess. Then hand over work that runs through a builder/reviewer loop and count what reaches the conversation: the thing landing, and not one line about the gates |
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
  substrate has a consumer. The undecided half is gone: threads open `ephemeral: false`
  ([`codex/process.rs`](../src/foundation/codex/process.rs)) and `thread/resume` is how Reaction
  and Cognition come back at boot. What replay would still add is reading a *finished* session's
  frames, which nothing does.

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
end to end, and both now arrive as one `Message` the boundary mints once. Vision journals and
wakes as *perception*, which is a kind of its own and not conversation; a handed file arrives as
its own message carrying its name and its locator in fields rather than in prose. What is absent
is push.
