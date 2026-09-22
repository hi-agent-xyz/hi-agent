# Legibility

## Goal

What a person reads is the product: every message the agent sends, every view it puts on
screen, every line on a task's record. The reader's attention is small and the writer cannot
notice its own defaults, so good output is not something a prompt can simply request. It has to
be held by a mechanism that fixes the inputs, checks the output, and learns from the reader.

**What counts as good** — the information model, the two variables, how to say it and in
what form, and the reasons — is [`../human-friendly-communication.md`](../human-friendly-communication.md).
This document is how the system keeps it true.

## What it is bound to

**The standard is bound to a class of artifact — text a person will read — and never to a
verb.** That distinction is the design. Getting it wrong is what the 2026-09-18 audit of one
live store found: every stage below sat on `hi_say`, so the mechanism covered exactly what
that one verb carried, and the surface beside it was untouched. Of 211 task records, 197
carried project vocabulary the reader has no copy of (`挂屏`, `写手`, `口径`, `ref`), 144
carried machine timestamps, 80% of 2,017 timeline lines ran past the length a card can draw,
the longest was 1,807 characters, and **41 of the 41 records opened in the four days before
the audit did all of it** — while the rules they broke stood written out in three separate
prompts, one of which quotes the previous measurement of the same failure. Nothing was
ignoring the standard. Workers reach `facet.md` with `apply_patch`, so there was no seam for
a standard to be held at, and a rule held nowhere is a rule the system does not have.

Three things bind, and they are the same three on every surface:

1. **One seam.** Everything a person reads leaves through a verb the host can see. A surface
   with no verb cannot be triaged, checked, audited, counted or replayed, and no amount of
   prompt substitutes for one — that is what the measurement above is.
2. **One standard, one judge, one record.** [`craft/reading.md`](../../src/identity/craft/reading.md)
   is the single text every writer writes against and every reader-of-writing judges against;
   [`judge.rs`](../../src/body/legibility/judge.rs) is one model request carrying it
   as a cacheable prefix; every judgment lands in `memory/quality/` **naming which surface it
   was on**. One record rather than one per surface, because a correction the person makes
   about how they are spoken to is a fact about *them* — "以后简要汇报" is supposed to shorten
   their task lines too, and today it cannot reach them.

   **So the check stops being Reaction's.** It sits under
   [`body/reaction/legibility/`](../../src/body/reaction/legibility/) because speech was the
   only surface it had; a worker calling `hi_task_note` reaches the host through MCP and not
   through Reaction's mouth, so the module moves to where every seam can call it. What stays
   Reaction-shaped is the triage (§ D), which reads a *turn* — each other surface brings its
   own facts to triage on.
3. **Judged at the seam, gated at some of them.** Every seam records a judgment. A seam also
   *gates* — answers `not sent`, `not recorded`, and lets the writer try once more — only
   where the writer can still act on the answer and something is waiting on the text. Short
   and frequent gates. Long and rare is judged after it lands, where a gate would buy a wait
   and nothing else.

A new human-facing surface ships with its seam or it does not ship
([invariant 13](arch.md#invariants)).

## The surfaces

| What the person reads | Who writes it | Seam | Judged |
|---|---|---|---|
| A spoken message | Reaction | `hi_say`, and a line prepared for their next message through `hi_prepare` | at the seam, gated — § A–K; a prepared line when it is prepared, not when it runs ([agents.md § *Prepared branches*](agents.md#prepared-branches)) |
| A task's title and its `created` line | Cognition | `hi_task_open` | at the seam, gated — § L–M |
| A task's timeline line | any worker | `hi_task_note` | at the seam, gated — § L–M |
| A view on screen | a view builder | `hi_view_verdict` — the view reviewer's | by the view reviewer, its verdict kept — § *Views* |
| Home's group labels and notes | a task manager | `hi_set_home_groups` | at the seam, gated — § *Home* |
| A report's wording, where it reaches the person | Cognition, workers | `hi_say` — the speech it becomes | where it becomes speech — § B, § E, § G |
| A file handed over (a report, a deck) | any worker | — | § Open |

**A report to Reaction is not a surface of its own.** A model reads it, so it may be complete
(§ B); what the person reads is the speech made from it, and that passes through `hi_say` like
any other. The row is here because a report's wording is the most common way machinery
reaches a message, and it is held where it reaches them rather than where it was written.

**A file handed to the person is the one row with no seam**, and it is why this is a table of
surfaces rather than a list of checks: the gap is found by enumerating what a person reads,
never by enumerating what the host already intercepts.

## The speech path

```
                    ┌──────────────── in Reaction's window every turn ────────────────┐
  what the person   │  A. who they are and how they want it                            │
  wants             │     conduct (Working with them) · per-subject grain · the standard│
                    └───────────────────────────────┬─────────────────────────────────┘
                                                    │
  Cognition/worker ── B. material, not a script ──► C. Reaction writes the turn
  (view built here when                                        │ hi_say
   the result will cross the line)                             ▼
                                             D. triage (host code, facts only)
                                                 │ in scope          │ out of scope
                                                 ▼                   │
                                             E. pre-send check ──────┤
                                                 │ revise            │ pass / timeout / error
                                                 ▼                   ▼
                                     "not sent — <note>"       F. delivered
                                     rewrite or drop,               │
                                     same turn                      │ turn ends
                                                                    ▼
                                             G. audit (independent, per turn)
                                                 │
                        ┌────────────────────────┼──────────────────────────┐
                        ▼                        ▼                          ▼
             H. Reflection learns      I. the number               J. replay set
             grain · conduct           corrections, findings        ── K. a prompt or
             (feeds A)                                                   model change runs
                                                                         here first (feeds A–C)
```

Each stage exists because the one before it misses something measurable.

## Speech, online: one turn

### A. What reaches Reaction's window

Reaction cannot open a file, so what it knows about the reader is what is projected into its
window every turn.

- **Conduct** — each person's `## Working with them` section
  ([`conduct.rs`](../../src/mind/memory/conduct.rs)). **The people in the conversation come
  first and are never the ones cut.** Ordering by name put the owner's section last behind
  eight others, and a 3,000-character cap across all of them cut it mid-sentence: on
  2026-09-15 the sections totalled 16,425 characters and Reaction received 3,023. A durable
  "report briefly" that never reaches the window is a preference the system does not have.
- **Per-subject grain** — the read Reflection already keeps on what the agent's words have
  earned, subject by subject (`proactivity.md`, projected by
  [`snapshot.rs`](../../src/mind/memory/snapshot.rs)), widened from *when to speak* to *how
  much detail lands on this subject*.
- **The standard** — one written standard, `src/identity/craft/reading.md`, distilled from
  the principles document, and the one text every writer and every judge is written against.

  **A rung that writes for a person carries it whole; it is never handed the path.** Reaction,
  Cognition, and every worker type that writes something a person reads — the general worker,
  the view builder and reviewer, the decision maker, the task manager
  ([`worker_prompt`](../../src/identity/mod.rs)) — close their system prompt with the page. A
  rule held on the condition that a model goes and opens a file is the arrangement 41 of 41
  records broke; the page is 12 KB against windows that run past 100K at a ~99% cache hit.

  **What each prompt keeps is how the page applies to its surface, which is not a copy of
  it** — a line on a record is one line, a view has one first landing point, a verdict names
  what to change. The page says what a person can take in; a prompt says what that means for
  the thing it writes. The sign that one has become a copy is a sentence the page already
  says, and those are deleted.

**Catches:** a reader's preferences and grain being unknown at the moment of writing.
**Misses:** a writer that has them and still does not apply them.

### B. What Cognition and workers send Reaction

A report to Reaction is read by a model, so it may be complete — but its wording tends to come
out of Reaction's mouth. The contract, in `cognition.md` and the worker prompts:

- what changed for the person first; backing detail marked as backing;
- no drafted wording ("say this", "照这个说") and no list of what was not done;
- **timing intent stays** — "hold this until the other half is verified" is information, not
  wording;
- **a result that will cross the view line gets its view built alongside it**, and the report
  names the ref. A view takes minutes; decided at speaking time, the only options left are a
  wall of text or a wait. When pieces of one subject accumulate past the line over several
  reports, the rung holding them has them gathered into one view.

**Catches:** relay of report detail and scripted phrasing at its source — 32 of 96 reports
Cognition sent Reaction between 09-11 and 09-14 carried wording for it.
**Misses:** Reaction's own habits: replaying the person, repeating itself, narrating process.

### C. Reaction writes the turn

It applies the standard with the two variables: how much clears the bar given what else is
landing, and the grain for this person on this subject. It decides how much to say (a
sentence, a paragraph, a few paragraphs — one matter per message, conclusion first) and whether
a view goes up with it.

### D. Triage — host code, facts only

Inside `ToolSink::say` ([`tools.rs`](../../src/body/reaction/tools.rs)), after the length and
the run count (§ F) and before the floor — a read that can take seconds must not hold a floor
decision made about the moment the words are ready. It judges nothing; it decides whether the
check runs. A message is in scope when **the turn carries a report**, or **the message is
longer than a short reply** (starting line: 120 characters), or **it is not the first message
of the turn**.

The scope is set by where the failures concentrate, not by how well code can find them: code
cannot tell a paraphrased repetition from new content (measured: routing 24% of messages to a
check caught half of the problem messages, 89% caught 95%). Short first replies to the person
go out untouched and immediately.

**What it passes is recorded, as a `skipped`** — because "not read" is a verdict. Triage judges
no wording, but a message it does not route goes out exactly as if a judge had passed it, and
left unrecorded those writes are missing from the denominator of every number in § I rather
than present in it. Measured 2026-09-20 on five days of this install: 154 messages went out and
115 were routed, so a quarter of the surface was outside every number being read off it — and
on records, where the audit reads only the close, a skip is final.

### E. Pre-send check — typed questions to System One

**What it is.** One System One call from the host — no session, no memory — through the
[decision capability](../../src/body/capabilities/decision.rs) `hi_system_one` also uses
([`check.rs`](../../src/body/reaction/legibility/check.rs)). The state is the case: who is
reading (the conduct and words-earned blocks Reaction's window carries), the recent
conversation, what was on their screen, this turn's incoming signals, the messages already sent
this turn and what it put on screen, then the candidate. The questions are one `noul` per axis
of the standard's table — `unsaid` excepted, since one message cannot show what the turn will
still say — and one `choice` over `pass` and those axes. Their wording is
[`judges/check.md`](../../src/identity/judges/check.md) around each axis's line from the table
as installed, so the table stays the only list of axes. The message is sent back when the
choice puts less than `speech_check_pass_below` (**0.5**) on `pass`, on the axis the rest of
the mass leans to most. **Every number the reply carried is recorded with the check**, so the
cut can move without asking any message again.

**Why not a model writing a verdict.** It did not answer. Five days on `deepseek-flash`
recorded 187 checks and 162 of them timed out, p50 22 s, because a verdict is ~50
tokens behind 900–1,500 of thinking. Asked the same axes as typed questions, System One
answered all 167 audited messages on this install at p50 0.53 s, p99 2.2 s. What that costs is
the note: System One writes no words, so it cannot quote the ones that fail.

**What it does with a revise.** `hi_say` answers `not sent — <note>`, the same kind of answer
as *too long* or *they were still talking*. Reaction reads it inside the same turn and
rewrites or drops the message; no new turn is woken. The note is the failing axis's own line
from the table — the standard the writer already holds, not a sentence for it to repeat.

**Limits that keep it from doing harm:**
- **one send-back per turn** — whatever Reaction sends after reading one goes out as written,
  as the floor lets a reply through after repeated refusals. Per turn rather than per message
  because the host cannot tell a rewrite from a new message, and a turn is what bounds the
  latency;
- **a timeout, and any error, sends the message** — silence is the worst failure, and a
  checker must not be able to produce it. Time spent queued behind an earlier message counts.
  The budget is `speech_check_budget_ms`, **2.5 s**, and the record gate's is 20 s (§ M): nobody
  waits on a line. Both were 2.5 s until 2026-09-20, when five days of recorded checks held
  131 verdicts and not one inside the budget — a reasoning model spends 900–1,500 tokens
  thinking before ~50 tokens of verdict, so the cheapest real case on the fastest model this
  install has takes 3.3 s. **A budget nothing can meet records `timeout` for everything and
  teaches nothing**, which is what it did; the speech check's went to 10 s for that judge and
  came back to 2.5 s with System One, which went past it on 1 of 167. Its upstream was seen
  unavailable for about a minute and a half on 2026-09-21 (503 and 529): what an outage costs
  is messages that went out unread, never messages held;
- **serial within a turn**, so order holds; a message that had already reached the mouth when
  an earlier one was sent back goes back with it, since it may depend on it — and one that
  arrives after the answer is the rewrite;
- **one switch** — `speech_check` set to `off` reads nothing; otherwise every in-scope message
  is read, and one that fails is sent back. Every check is recorded with what it cost and what
  it answered (§ I), so what a different cut would have done is read off the checks that ran.

**Catches:** what can be read off the words — a claim of something on screen with no show this
turn (09-15: "it's on screen" a minute before it was), a message that announces it repeats.
**Misses:** most of what the person means by a bad line. Against 20 messages they labelled on
2026-09-21, its ranking of their send-backs was no better than chance (AUC 0.44–0.55): a
read-back of their own instruction, a promise to report back, awkward or packed wording all
mostly passed. The model it replaced reached 57% precision and 44% recall against the audit on
91 messages and was never measured against the person. **It is taken for the number it
produces, not for how well it judges** — a check that answers is one there is something to
learn from and to build better logic on. It is a net, not a guarantee; and it cannot read
messages later in the turn or problems that only exist across turns.

**Does not:** write words for Reaction (that would be a second mouth), or block on anything
the person must do.

### F. Delivery

One message is one matter. Paragraph breaks are part of the text: the face keeps them
([`Chat.tsx`](../../src/appearance/web/src/ui/Chat.tsx)) and the speech splitter cuts on them
([`segment.rs`](../../src/foundation/segment.rs), in agreement with `sentences.ts`).
`SAY_MAX_CHARS` is the size of one matter; too long means say less, never send it in pieces.

**Three messages go out between one of the person's and the next.** Everything sent since
their last message is read at once by someone coming back to it, with every subject that
moved in the meantime interleaved, so the run is what the standard's bar applies to
([`reading.md`](../../src/identity/craft/reading.md)) — and the host holds its length. In
`ToolSink::say`, after the length and before D, a fourth message is refused with `not sent`
([`unanswered.rs`](../../src/body/reaction/unanswered.rs)). A message from the person — typed,
spoken, or a handed file, the inputs that become messages — starts the run over; nothing
else does. The count is taken under the mouth's serial lock, and a loop standing up seeds it
from the run the journal already ends in, so a restart hands out no fresh allowance.

Where each piece of work got to is on its task row, one subject apart from the next and only
its newest state, which is what somebody catching up can read. A row `waiting` on them reads
*Needs you* as its status word on Home as well as on the board ([`home.md`](home.md)), and
Reaction owes it first when they next write.

**Catches:** the pile. From 08-26 to 09-17 on one install, with every typed, spoken and
handed line counted as theirs, 850 of 1,189 agent messages (71%) sat in runs of four or
more, the longest 58; on 09-17 a run of 19 in 81 minutes interleaved six subjects, one card
number corrected three times, and each subject had its own row saying where it stood.
**Misses:** what goes in the three, which is C's. And something that needs them arriving
after the run is full waits for their next message, or for them to look at Home.

## Speech, offline: learning from what was sent

### G. Audit

**When:** a Reaction turn that spoke ends; and again when the person's next message arrives,
because that message is the ground truth for whether the turn landed. Event-driven, never
periodic.

**Reads:** the turn's full input (from the frame log), what was sent, the pre-send verdicts,
the person's next message, and the reader's conduct and grain as they stood.

**Writes:** one JSON line per judgment to `data/memory/quality/<day>.jsonl`
([`quality.rs`](../../src/mind/memory/quality.rs)), three kinds:
- a **check** — each pre-send verdict, its scope, mode and latency;
- an **audit** — each message's axis or none, anything owed and left unsaid, anything wrong
  ([`judges/audit.md`](../../src/identity/judges/audit.md));
- a **reception** — whether the person's next message corrects *how* something was said, the
  axis, and their words ([`judges/reception.md`](../../src/identity/judges/reception.md)). It
  reads the spoken turns since their previous message, the latest four at most.

Whether the check agreed is computed from the records, not asked: the audit never sees the
check's verdict, so it stays independent. The `speech_audit` setting turns both reads off;
`speech_audit_model` picks their model.

**It reads a spoken turn and nothing else.** A view the turn put on screen and a task line a
worker wrote in the same minute are not in it, and were never in it: the audit is keyed on a
turn, and a worker's write is not part of one. § N is the same read pointed at a record.

**Catches:** what the check cannot — dropped answers, repetition across turns, a subject that
should have become a view, the check being wrong.
**Misses:** nothing already sent can be recalled; its value is the next turn and the next
change.

### H. Reflection learns

Reflection already reads the stream and keeps the per-subject read and the people facets. Each
settling pass is shown the corrections and the findings recorded over the stretch it is
settling — **on every surface, each labelled with the surface it was on**, because a lesson
about how the person wants to be told things is not speech's alone and a task line read as a
"spoken message" would teach the wrong thing — beside the signals, and folds them in:

- **grain per subject**, into the words-earned read — a question raises the grain for that
  question only; a correction ("以后简要汇报") changes the subject's standing;
- **durable conduct**, into the person's `## Working with them` when they state a lasting
  preference about how they want to be told things.

Both reach Reaction through A. Nothing new is stored beside them.

### I. The number

Server-side, beside the logs; no card in the face. `GET /api/legibility?days=7` computes them,
one set per surface — `speech`, `record`, `view` and `home` — from the records on read.

| Measure | Why |
|---|---|
| **Corrections of how something was said, per day** — the primary | the reader is the only authority on the reader's bar; target 0 |
| audit findings per 100 messages, by axis | where the failures are |
| owed and left unsaid, per 100 turns | so shorter never passes for better |
| pre-send revise rate; latency p50/p95; timeouts | whether the source is getting fixed, and what the check costs |
| **the share of writes a judge read at all** (`read_rate`, checks over checks + skips) | a revise rate over the routed alone describes the sample, not the surface |
| tokens per verdict, and how many of them were thinking | latency here is almost all output tokens; without this a slow judge and a slow network read the same |
| check–audit agreement | whether the check deserves its place |
| writes around the seam, per surface — with who was on the row and how much moved | the record seam is held by detection, not a wall (§ L); this is the number that says whether it is holding, and the fields are what make it answerable |

### J. Replay

Every Reaction turn's complete input is in the frame log's `turn/start` frames, so a past
turn can be run again under a changed prompt or model with tools stubbed, and scored by the
audit. `make eval-speech` ([`replay.rs`](../../src/body/reaction/legibility/replay.rs)) does
this against a set drawn from three sources: turns the person corrected, turns the audit
flagged, and turns judged good (so a fix cannot overshoot into leaving things out); with no
records yet, the most recent turns that spoke.

Each turn is replayed with its thread's opening turn (which carries the whole window) and the
six turns before it as history, under this build's prompt — or `PROMPT=`, on the agent's model
or `MODEL=` — and `hi_say` answers the way the host would, floor aside, including the run
since the person's last message as the journal had it when the turn started. That is not the live
thread; codex's compactions are not reproduced. Both sides of the comparison are scored by the
same audit, which is what the question needs. The set is private conversation, and the report
stays in the data directory, under `memory/quality/replay/`.

**A record is replayed at the moment it was written.** `make eval-records` takes worker turns
that called `hi_task_note`, puts in front of a model everything that turn had seen up to the
call — its brief, and what its commands and tools returned, as a transcript — under this
build's worker prompt or `PROMPT=`, and asks for the line again with `hi_task_note` the only
tool. Both lines are read by the record audit. The rest of a worker's turn — its shell, its
builds — is not reproduced, and does not need to be: the question is whether a change writes
a better line from the same material. The set is drawn the way speech's is: lines the gate
flagged, lines it passed, and with no records the most recent.

### K. Changes go through replay first

A change to `src/identity/`, to the standard, to the check's prompt, or to the model Reaction
runs on is replayed against the set before it lands, and lands only without a regression on
the primary measures.

**The runtime is one of the things replay compares.** In the 2026-09-14 rehearsal, the same
`reaction.md` run by a different model with a short window cut judged-problem messages from
108 to 15; over those days the live Reaction ran `deepseek-flash` with 75K–236K input tokens
per turn. Model and context length are levers of this design, not background.

## Task records, end to end

What a person reads of a record is not the file: [`tasks.jsx`](../../src/mind/views/factory/tasks.jsx) draws the title and **the newest
line said on the row — a mind's, or the person's own reply — clamped to one line**, on the card, and behind it a panel that is a head and
one list. The head is the status word, the title, who is on it, and the single worst thing wrong
with it — *Needs you*, overdue, nobody on it, and only the worst. Under it the whole record, newest
first, and nothing above it: the standing prose that used to open the panel went with the
account. **The ask and the wait are entries in that list, and the panel draws neither of them
twice**: `created` is its oldest entry and a `waiting` line its newest. The row's machinery —
the frontmatter this schema does not parse, a duty's check, the file's own name — is one fold
at the bottom. Home draws the title again. A title, a line and a paragraph are three reads with
three different budgets, so the seam has to be able to tell them apart.

### L. The seam — three verbs, and the host owns the file

Cognition opens a row and every worker appends to it with `apply_patch` on `facet.md`
([`general.md`](../../src/identity/workers/general.md)), which is why nothing in § D–G can
see any of it. Three verbs replace that, and `## Timeline` stops being hand-patched:

    hi_task_open(subject, title, status, wanted, …)            Cognition
    hi_task_note(kind: update | delivered | waiting | title, text, subject?)             workers
    hi_task_set(status?, due_at?, checked?, verify?, …, subject?)            workers

**Prose is judged; machinery is validated.** The first two carry sentences a person reads
— a title correction included, since the title is on every card — and are where § M sits. The third carries the row's machinery — the status word, a due
date, a duty's `verify` / `restart` / `owner` / `start_key`, the stamp that its check came
back alive — which is read as values, not sentences, and which `reconcile` could always
repair from the bytes. It is a verb anyway because without one a worker keeping a duty's
`checked_at` has to open the record to do it, and the file open under its cursor is the one
it patches the timeline in by habit. `subject` defaults to the task the calling worker
serves. **Folding** a duplicate row into the one it duplicates is `hi_task_set(fold_into)`:
the store carries the folded row's account and every line into the survivor, merged by the
instant each was written — which a mind re-typing them through `hi_task_note` would have
re-stamped as now — and closes the folded row saying where the promise went.

- **The host writes the instant and the kind**; a caller passes prose and nothing else. That
  removes the corpus's most common failure by construction rather than by rule — 2,959
  machine timestamps across 144 records, in the prose because hand-writing the whole line is
  what the format asked for.
- **Every kind is one line, because the account is gone.** `stands` was the one that was not:
  it put new prose on top of a body and pushed the last reading down, never dated and never
  dropped. See § *Decisions*.
- **`created` is written once, at open, by `hi_task_open`**, so the acceptance line and the
  title are one call by the rung that was in the conversation. A second `created` is refused.
  Nothing enforces the "once" today and nothing reports its absence: of 106 rows in one
  store, three had one.
- **`moved`, `made` and `replied` are not in the enum.** The store writes them: the first two
  about things it witnessed, the third the person's own words from the row's reply box
  ([`data.md`](data.md#tasks)). § M does not read that one — its standard is for how the agent
  writes to them, not how they write back.
- **Nothing else writes the frontmatter or `## Timeline`.** A worker's own working files in
  the task folder are unchanged; the record is the host's.

**Deleting the hand-patched path is part of this change, not a follow-up.** Two ways to write
a record is a door beside the check, and a compatibility path kept until the prompts catch up
is the one that stays.

**And the seam is held by detection, not by a wall — say so rather than imply otherwise.**
Cognition and workers run `danger-full-access`
([`process.rs`](../../src/foundation/codex/process.rs)), so a verb cannot stop anyone from
reaching `facet.md` with `apply_patch`; a sandbox that could is a separate decision with its
own cost. Two things carry it instead:

- **The record moves out of the folder the worker writes in**, to `memory/tasks/<subject>.md`.
  The task folder (`memory/facets/tasks/<subject>/`) keeps the deliverable and the working
  notes; the record is no longer sitting among the files a worker edits all day. That
  overturns *a task is a facet* and is stated where it was decided
  ([`data.md`](data.md#tasks)).
- **The store holds what it last wrote.** A record that changed by any other route is a
  finding in `memory/quality/` under the same surface, counted in § I. The pass that does
  this exists: [`reconcile`](../../src/mind/memory/tasks.rs) already re-reads every record on
  every window build and already keeps a `LAST_SEEN` mark per row, which is why detection is
  an extension rather than a mechanism.

  **A detection carries who was on the row and how much moved**, not only that something did.
  The first shape of it held the instant, the surface and the subject; asked on 2026-09-20
  which session had written twenty-one lines around the verbs on one row, it could not answer,
  and the frame logs took half an hour to half-answer. So the record names the sessions the
  index has opened against that subject — a short list to look at, never proof, since anything
  unsandboxed can write any file — and the record's size before and after, because one line
  appended by hand and a record rewritten whole are the same fact to a hash and are not the
  same event.

**This overturns a decision `reconcile` was written under, and it is worth saying which
half.** Its reasoning — *the ledger has no code on its write path and should not get one; a
verb an agent could be asked to use is a door beside an open wall, absent exactly when it is
forgotten and silent about being absent* — is correct and is why the two measures above
exist rather than a verb alone. What it did not weigh is the difference between the two
things in a record. **Mechanical fields can be repaired by a pass that re-reads the bytes;
prose cannot.** No later pass can turn a line written badly into the line that should have
been written, because the context that would have produced it belonged to the session that
wrote it and is gone. That is the whole reason this surface needs a seam and the status word
never did: a check is only worth having while the writer can still act on it.

That is weaker than a wall and much stronger than the prompts that produced 41 of 41: a
hand-patched record becomes a countable defect instead of an invisible one, which is the
whole difference this design turns on.

### M. The gate — on a line, not on a record

A line is short, the worker writes it holding everything it is about, and the card can draw
one line of it. All three say gate. The rules are § D–E's unchanged, because they were
derived against the same problem:

- **Triage in code, facts only**: past a length, or `kind: waiting`. The line that asks the
  person to act is the one whose burial costs most — on one store the longest ran 920
  characters with the ask in the middle and a second, unrelated ask behind it. Everything
  else is recorded untouched.
- **One model request** — the same [`judge.rs`](../../src/body/legibility/judge.rs),
  a rubric in `judges/record.md`, the standard as the cacheable prefix, plus who the reader is
  (their conduct) and the row's recent lines, without which "the same thing said again" is
  invisible.
- **`not recorded — <note>`, once.** The worker rewrites, and the second attempt is written
  whatever it says. Speech can be dropped; a record cannot. An unrecorded fact is worse than
  an ugly one, and a gate able to lose facts would be a worse failure than the one it fixes.
- **Fail open** — `record_check` set to `off` is its one switch, as `speech_check` is
  speech's. A timeout writes the line. Its budget is `record_check_budget_ms`, **20 s** — twice speech's,
  because the only thing a record gate holds is the worker writing the line (§ E for why both
  moved off 2.5 s).
- **What triage passes is recorded as a `skipped`** (§ D), and on this surface that matters
  more than on speech: an ordinary short line is read by nothing afterwards either, because
  § N reads only the close.

### N. What is read after it lands

One read of the whole record when a task manager closes it — the moment the artifact is
finished — judged after the write rather than before, because nothing waits on a record that
is already written. Same judge as the line gate, same standard, findings to `memory/quality/`
under `surface: record`, nothing sent back. **There was a second read, on every rewrite of a
task's account**; it went with the account, and with it a judge call per write.

**The closing read also answers what the record does not say**, against what the sessions that
served the row reported: every message they sent their owner, found by joining the mail log
(`raw/sessions/mail.jsonl`) to the session index's `subject` (`raw/sessions/index.jsonl`).
Their own account of what happened is the one source that was in the room, so what it says
changed something for the person and the record never carries is *owed and left unsaid*. A gate on length is an incentive to write fewer lines, not shorter ones, and
a line never written is invisible forever — worse than the long one it replaced, because a
record's whole job is that somebody downstream was not in the room. Speech already carries
this counterweight (*owed and left unsaid*, § I, so shorter never passes for better); the
record surface needs its own or the gate will be measured as a success while making records
thinner. It is the one number to watch first on this gate.

The read's rubric is `judges/record_audit.md`; the `record_audit` setting turns it off and
`record_audit_model` picks its model, as speech's audit does. What it writes is an audit like
speech's — one entry per thing read, anything owed and left unsaid, anything wrong — so § I
counts it, § H learns from it and § J scores with it without a second copy of any of them.

### Views

The builder writes against the standard and against **who the view is for** — a report for
the person to review may mark what is unverified; something made to be shown to others (a
deck, a shared page) carries no working notes, which go in the conversation. The reviewer
judges against the same standard, and **its verdict is kept** in `memory/quality/` under
`surface: view` like any other judgment — recorded through `hi_view_verdict` (`ship` or
`not_yet`, the axis and the finding), refused at dispatch to every worker but the view reviewer — a verdict nobody keeps is one nothing counts and
nothing learns from, and a view that reads badly would never appear in § I. The reviewer is
the view's judge already; what this surface needs is its answer recorded, not a second
checker. The spoken line that goes with a view passes D and E like any other message.

## Home

A group's label and its note are on the person's home screen every time they open it, and are
written by one task manager in one call that replaces the whole arrangement — so the gate is
the record's (§ M), pointed at names: every label or note that is new or changed since the
standing arrangement is read, with `judges/home.md` and the reader's conduct, before it lands.
A label is a name in the person's own words; a note is one line on what the grouping was based
on. `home_check` is its switch and `home_check_model` the model; a refused
arrangement is answered `not arranged — <note>` once, and the next one lands whatever it says,
because an arrangement the person asked for is not held back over its wording.

## Decisions

| Decision | Reasoning |
|---|---|
| **A pre-send check, scoped to report turns and long or later messages** | Code triage cannot find paraphrase, so scope follows where failures concentrate. A small-model checker found ≤11% of problems and is not used; mid-size reached 57% / 44%. On report turns a few seconds are affordable, and those turns carried the worst messages on 09-15 |
| **One send-back per turn, not per message** | The host cannot tell a rewrite from a new message; a turn bounds the latency, and what follows a send-back was written with its note in hand |
| **The judges' rubrics are prose in `src/identity/judges/`** | What counts as a failing line is judgment, and judgment lives where it can be read whole; the code only sends it |
| **The check can send back, never rewrite** | A checker that edits words is a second mouth ([invariant 1](arch.md#invariants)); `hi_say` already answers calls with refusals Reaction acts on |
| **Fail open** | A timeout or error sends the message. The one failure nobody reports is silence |
| **A gate is on or off; there is no recording-only stage** | Every gate was built to record what it would have done and send nothing back until its numbers were known. Five days of that on speech kept 187 checks, 162 of them timeouts, and no verdict ever reached anyone. The checks that run are recorded anyway, so a stage that only records is the same numbers plus a second path (2026-09-21) |
| **Conduct: people present first, never cut** | Name order plus a shared 3,000-character cap delivered 3,023 of 16,425 characters and cut the owner's own section |
| **Grain lives in the existing per-subject read** | Reflection already learns what the agent's words earn per subject; a second store would be structure with the same job |
| **No derived load score** | The bar's float is judged from facts in the window; the host's presence estimate was deleted because nothing real could produce it ([`host.md`](host.md)) |
| **One matter per message; structure is paragraphs** | Supersedes "three short messages" in [`text-transcript.md`](text-transcript.md) |
| **Three messages between one of theirs and the next, refused in host code** | The person asked for a low cap (2026-09-17). The rule was already in `reaction.md` — "one quiet word beats a string of pings" — and 71% of messages still sat in runs of four or more, so the prompt alone has been measured. The count is a fact about the conversation, the same kind as length, and the refusal names it without judging the words |
| **No exemption for urgent, and no backstop** | A flag the writer sets for itself would be set on everything. What needs the person is a `waiting` line on its row, drawn as *Needs you* where they come back to, and first when they next write. The floor lets a reply through after repeated refusals because silence is its failure; three messages already standing are not silence |
| **The primary number is the person's corrections** | The rehearsal's blind judge marked as *dropped* an item the person said was right to leave out; labels drift toward completeness |
| **Changes are replayed before they land** | Two prompt changes without a measurement between them cannot be told apart |
| **Bound to the artifact class, not to `hi_say`** | A mechanism wired to one verb covers what that verb carries. Beside a speech path with triage, a check, an audit, a number and a replay set, 41 of 41 task records opened in four days carried jargon and machine timestamps, against rules written in three prompts |
| **One record for every surface, carrying which surface it was** | A correction about how the person is told things is a fact about the person. Kept per surface, "以后简要汇报" teaches speech and leaves their task lines alone |
| **The standard is carried whole, never linked** | Four rungs were handed the page's path and asked to open it — a rule held on the condition that a model remembers to go and read a file. The records written under that arrangement broke it 41 times out of 41 |
| **The host writes a record's instant and kind; a caller passes prose** | 2,959 machine timestamps across 144 records were written into prose because hand-writing the whole line is what the format asked for. Removing the ask removes the class |
| **Prose is judged; machinery is validated** | The status word, a due date and a duty's liveness fields are values `reconcile` could always repair from the bytes; a sentence is not. `hi_task_set` exists anyway, because a worker that must open the record to stamp `checked_at` is a worker with the timeline under its cursor |
| **A refused record line lands on the second attempt; a refused message does not** | Speech that is dropped is silence, which the floor already answers for. A fact that is not recorded is gone, and a gate able to lose facts is a worse failure than the one it fixes |
| **Gate the line, judge the record at its close** | A gate is worth a wait only where the writer can still act on it and something is waiting on the text. Nothing waits on a record already written, and a builder must not queue behind a judge |
| **The account is deleted; the record is the whole of what a row says** | `stands` prepended and nothing ever dropped a reading, so it grew without bound (median 2.2 KB over 212 records, largest 58 KB), went stale at the top — `buried` was a verdict for exactly that — and, sitting at the head of the file, took the whole of `work_record`'s 3,000-character budget: **zero timeline lines reached the worker on 36% of records**, a median of 3 of the 8 written. What it was for, a standing summary, is what the newest lines already say, dated and in order. Prose on records written before the retirement is round-tripped and surfaced nowhere: deleting a mechanism is not licence to delete what somebody wrote |
| **The hand-patched path is deleted, not deprecated** | Two ways to write a record is a door beside the check, and the compatibility path kept until the prompts catch up is the one that stays |

## Open

- **Lines**: the triage length (120 characters), the message ceiling (400), the run between
  their messages (3), when a matter becomes a view — all starting values for the replay set
  and the person's corrections to settle. The record line's triage length is the same kind of
  starting value, and the card draws roughly one line of it. So is the check's cut on `pass`
  (0.5), with the difference that every check keeps the numbers it was cut from.
- **What the check should ask.** § E asks the table's axes one by one, and that is what System
  One reads worst: the person's own reasons on 2026-09-21 were "uncomfortable to read" and
  "wordy — the last sentences add nothing", which the table spreads over `hard`, `known` and
  `repeat` or does not name at all. Better questions over the same judge are the next work here,
  not a different judge.
- **The surface with no verb: a file handed over.** A weekly report, a deck, a page written
  into the drive for the person to open is read by them and passes through nothing. Whether
  the seam is a verb that hands a file over, or the delivering `hi_say` carrying the ref, is
  undecided — and until it is decided, this is the one row of the table that the invariant
  cannot yet be tested against.
- **Whether a record's judgment should reach the person at all.** § I counts it and Reflection
  learns from it. Nothing draws it on the board, and a row that showed its own writing quality
  would be the agent grading itself where the work should be.
- **Whether a full run should reach them some other way.** A `waiting` line written after the
  third message is on Home and the board, and neither pushes. If a need they miss that way
  shows up in the record, the answer is a channel that reaches them, not a fourth message.
- **Which model and how much context Reaction runs with.**
- **The grain for a subject with no signal** — coarse keeps attention, but a reader away from
  the window pays more to follow up.
- **Calibrating the audit's *owed and left unsaid*** so it excludes what the reader already
  knows and what resolves on its own.
