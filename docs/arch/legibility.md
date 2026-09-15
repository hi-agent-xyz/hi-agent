# Legibility

## Goal

What a person reads is the product. Every message the agent sends, every view it puts on
screen and every line on a task's record is spent out of one budget — the reader's
attention — and that budget is small: a person reads a few words a second, while they take
in a composed screen in parallel. Everything here serves one test: **did reading it cost
them less than it gave them?**

It is a first-class concern, not a style note, because the failure is invisible from the
writing side. Between 2026-09-11 and 09-14 the person corrected the agent four times for the
same thing — too much detail, too much process — while every rule against it was already
written in `reaction.md` (see *Decisions*).

## What is worth saying

A piece is worth saying when **its value to the reader clears the bar at that moment.**

**Value is departure from what they would already assume, at the grain they want for this
subject.** A reader carries default expectations, and restating one carries nothing:

| They already assume | So this carries nothing |
|---|---|
| work in progress is unfinished | "not read through yet", "not made into a list yet" |
| the obvious next step will be taken | "I'll show it to you once it's sorted" — especially their own instruction echoed back |
| a check that is not mentioned passed | the digest, the `200`, "restarts 0", which checks ran |
| what they already know is still so | a waiting item repeated with nothing changed |

What departs from those — a failure, a different route, a cost that grew, something only
they can do — is the news. An outcome stated once ("deployed, checked, fine") is the whole
of a good result.

**The grain is per person and per subject, and it moves.** The same person wanted "it's
live" for a deploy whose method they own, photo-by-photo detail on a birthday deck they were
editing, the source text when they doubted a platform limit, and background on a project
they had never worked on. What moves it is observable: the questions they ask (a *why* or a
*show me the source* raises it for that question), the corrections they make (*less detail
next time* lowers it for the subject), how they engage (editing item by item is fine grain).
A single question raises the grain for that question; only a correction changes the
subject's standing.

**The bar floats with how much is already competing for their attention — within a narrow
range.** When several things are landing, or they have said they are focused on one, less
clears it; when little is going on, a reason or a line of context can ride along. The range
is narrow for a structural reason: lowering the bar admits what is worth little, never what
is worth nothing.

**Compress by deleting, never by packing.** A short message that welds four clauses together
reads worse than the long one it replaced.

## How much, and in what form

Two separate calls.

**How much to say** is continuous: a sentence, a paragraph, or a conclusion with a few short
paragraphs under it — one matter per message, the parts of it in order, a line break between
them. That is the whole of text structure; no markdown, no numbering required. A message too
long to be one matter is refused (`SAY_MAX_CHARS` in
[`tools.rs`](../../src/body/reaction/tools.rs)); the answer is to say less, never to split it
across messages, which hides how much is coming and lets other messages land inside it.

**Whether to add a view** is a switch. Past a few paragraphs, or when the matter compares
several things on several properties, or when most of it is detail the reader will skip, it
is a document and belongs on screen or in a file. **The speech does not stop**: it says a
sentence, a paragraph or a few about what the view means, and never reads the screen out.

**A view is prepared where the content is produced, not where it is spoken.** A view takes
minutes; by the time Reaction holds the finished material, the choice left is a wall of text
or a wait. So the rung that produces a result that will cross the line has the view built
alongside it, and the delivery is the view plus a few words. **The same holds across turns**:
when one subject accumulates past the line in pieces — answers arriving one at a time over a
quarter of an hour — the pieces are gathered into one view rather than extended by another
message.

## Where each part lives

| Part | Home |
|---|---|
| Message shape | `hi_say`'s ceiling and description ([`mcp/mod.rs`](../../src/foundation/mcp/mod.rs)); line breaks kept by the face ([`Chat.tsx`](../../src/appearance/web/src/ui/Chat.tsx)) and cut on by the speech splitter ([`segment.rs`](../../src/foundation/segment.rs)), which must agree with `sentences.ts` |
| The standard | one written standard, `src/identity/craft/reading.md`, read by every rung that writes for a person — Reaction, the view builder, the view reviewer, a worker writing a task record. The copies of it now spread through those prompts fold into references |
| What Cognition sends Reaction | material, not a script: what changed for the person first, backing detail marked as backing, no drafted wording, no list of what was not done. **Timing intent is not wording and stays** — "hold this until the other half is verified" is information Reaction needs |
| The grain per subject | the per-subject read Reflection already keeps on what the agent's words have earned (`proactivity.md`, projected into Reaction's window — [`snapshot.rs`](../../src/mind/memory/snapshot.rs)), widened from *when to speak* to *how much detail lands* |
| The bar's float | Reaction's judgment from facts already in its window — the focus they stated, how much it has just said, how many things are landing. **No derived load score**: the host once carried a decaying estimate of presence and deleted it because nothing real could produce it ([`host.md`](host.md)) |
| Audit | after each Reaction turn that spoke, an independent read of what came in, what went out and the person's next reply, recorded per turn. Event-driven — a turn ending, a reply arriving — never periodic |
| The number | the count of times the person corrects *how* something was said. Beside it: audit findings per hundred messages by kind, and how often an answer owed to them was left unsaid, so that shorter never passes for better |
| Replay | a changed prompt or model is run against past turns before it lands: every turn's full input is in the frame log's `turn/start` frames, so a turn can be replayed with tools stubbed and scored by the audit. Replay sets hold private conversation and stay in the data directory |

## Decisions

| Decision | Reasoning |
|---|---|
| **No pre-send model check in the core** | Measured on 223 labelled messages (2026-09-14). A small-model checker found at most 11% of problem messages. A mid-size one reached 57% precision and 44% recall. Code triage cannot separate paraphrased repetition from new content: routing 24% of messages to the check caught half the problems, routing 89% caught 95%. So a check would sit on nearly every message at mid-size latency, for a minority of problems |
| **The model and context Reaction runs with are part of legibility** | In an offline replay of 117 turns, the same `reaction.md` run by a different model with a short window cut judged-problem messages from 108 to 15 and turns the person would push back on from 31 to 0. Over that stretch the live Reaction ran `deepseek-flash` with 75K–236K input tokens per turn. Prompt rules on top of the other model added a smaller gain (preferred 47–31, 39 ties) and dropped more owed items |
| **One matter per message, not three short messages** | Supersedes the earlier shape in [`text-transcript.md`](text-transcript.md) |
| **Structure is paragraphs** | A chat already renders paragraphs, and speech already reads them; markdown is a second syntax for the reader and the voice to undo |
| **The primary number is the person's corrections** | A judge's labels drift toward completeness: the replay's blind judge marked as *dropped* an item the person later said was right to leave out. The reader is the only authority on the reader's bar |

## Open

- **Where the lines sit**: the message ceiling (400 characters is a starting value), and when
  a matter becomes a view. Both want the replay set, not a guess.
- **Which model and how much context Reaction runs with.** The switch to `deepseek-flash` at
  2026-09-10 15:06 and the window growing past 200K tokens are the two largest measured
  levers, and neither has been compared on the real runtime yet.
- **The grain for a subject with no signal.** Coarse keeps attention; but a reader who is not
  at the window cannot follow up cheaply, so asynchronous delivery may want to start finer.
- **Judge calibration.** The audit's notion of *dropped* has to exclude what the reader
  already knows or what resolves on its own, or it will pull the agent back toward saying
  everything.
