# Signal attribution

**Status:** accepted August 13, 2026. New — nothing before this said who a signal came
from, and the gap had already put one person's words on another person's record.

## Decision

Every inbound signal records **who it came from**, as a field on the signal, decided at
the boundary where it arrives.

**Unknown is a value that field holds and keeps.** Not knowing who sent something is an
ordinary, expected state — most of what a room produces is unattributable — and it must be
representable, storable, and survivable. A signal whose sender cannot be grounded stays
unattributed for as long as that is true, exactly the way an unnamed voice cluster stays
unnamed.

Attribution is a property of **how a signal arrived**, never an inference from what it
says. Nothing downstream may derive a sender from content.

## Why this exists

Faces and voices already had this right. Clustering produces an opaque id — a real,
distinct person whose identity is unknown — the id persists, accumulates evidence, and is
named only by a deliberate act (`hi_name_person`, guarded by *"only name or merge when
you're sure — a wrong name sticks to a person"*).

Text and files had none of it. No id, no sender, no way to say "someone, I don't know
who". So a settling pass asked which person a stretch belonged to, had no grounded answer
available, and produced an ungrounded one — a `people/<name>` string in an episode's
subject list, written with no evidence requirement and indistinguishable ever after from a
name that was verified.

The failure is not that the agent did not know who was typing. It is that **not-knowing
had nowhere to go, so it became knowing-wrong**, and then hardened, because nothing
recorded that it had been a guess.

## Three classes of source

A channel already knows what kind of thing reaches it. That is the whole basis of the
rule — no new judgment is required at the boundary.

| Class | Channels | What arrival means | Sender |
|---|---|---|---|
| **Addressed** | `text`, `file`, `view` | someone deliberately sent this *to the agent* | the **owner**, by default |
| **Ambient** | `audio`, `vision` | captured from wherever the agent is | a cluster, or unknown |
| **Machine** | `clock`, `worker` | the agent's own machinery moved | **none, ever** |

The classes are not new vocabulary. `Channel`'s own definitions already draw these lines —
`file` is *"handed"* rather than perceived, `clock` *"came from no one, which is why it gets
its own channel rather than being mixed into `text` where it would read as something the
person said"*, `worker` is *"from another of its own minds rather than from the person"*.
Mis-sourcing was already considered serious enough to justify a dedicated channel. This
carries that one step further, to the signal.

**`view` was a machine channel and is now an addressed one** — amended August 19, 2026,
when the channel grew an inbound half ([`stage.md`](stage.md)). Outbound it is still the
agent showing something, which asks no sender question at all. Inbound it is the person
*going* to a view, on the agent's own surface, through a control nobody else can reach —
as deliberate an act as typing, and attributed the same way: the owner, basis `owner`,
defeated by any positive evidence. What it is not is machinery moving; nothing moved but
the person.

**Machine channels take no sender and are not a person's absence — they are a person's
non-involvement.** A stretch made entirely of worker reports and clock wakes is not a
stretch with an unknown participant; it has no participant. It teaches nobody anything and
must produce no person record at all.

## The owner

**One person owns an install.** This follows from *"one body per person"*
([`topology.md`](topology.md#identity)) — two machines running one handle would be one
identity with two memories, and by the same argument one install has one person whose
agent it is.

The owner is **declared, not inferred**, and lives in the config store beside the mode flag
and credentials. It names a `people/<subject>` facet.

**Declaring it is an act on a person, not a setting.** It lives on the 认识的人 review
surface (`POST /api/people/owner`), beside naming, ejecting and regrouping — because
what it does is point at somebody the store already holds and say *this one is me*.
Putting it in Settings would have made it a string to type, and a typed string that
matches no cluster is exactly the silent, unattributed install this section exists to
prevent; from the review page the candidates are on screen. A name the store has not met
is still accepted — a fresh install has met nobody, and the owner is often the first
person named — and the reply says which of the two it was.

**That one verb is loopback-only, and its neighbours are not.** Naming, ejecting and
regrouping are corrections: visible, reversible, and safe from a paired phone.
Declaring the owner is not a correction — it silently changes who every future typed
line is attributed to — so it takes the posture `/api/settings` already takes for mode
and credentials: the person holding the machine. A server install is unaffected, since
`curl` on the box is loopback and that is the same person who deployed it.

**It is read from the store on every addressed signal, not from the boot snapshot.** The
cognition tunables are loaded once at startup and apply on restart, which is right for a
parameter and wrong for an identity: the person who just said *this one is me* would go
on being unattributed until the process happened to come back up, with nothing on screen
saying why.

This does not reopen *"there is no user slot and no self slot"*
([`data.md`](data.md#prompts)). That rule is about **instructions** — a preference, a
correction, a standing request — and those still land as facets and tasks, going through
the agent's judgment like everything else the person says. The owner record is not an
instruction; it is an identity, the same kind of fact as the handle and the credential, and
identity has never been something the agent was supposed to work out for itself.

**An install may have no owner declared.** Then addressed channels are unattributed, and
that is a correct and complete answer, not a degraded one.

## The sender is recorded with its basis

The field carries **who**, and **how that was decided**:

| Basis | Means | Set by |
|---|---|---|
| `owner` | the addressed-channel default | the channel rule |
| `cluster` | a face or voiceprint matched | recognition |
| `stated` | the signal itself says who sent it | the carrier |
| `unknown` | not grounded | everything else |

**The basis is the load-bearing half.** A default that is *labelled a default* is
correctable — a later pass, a person, or a recognition can defeat it. An inference that is
merely written looks exactly like a verified fact and can never be told apart from one
again. That is the property that was missing, and it matters more than getting the default
right.

So `owner` is defeasible: positive evidence beats it. A voice recognized as someone else on
an ambient channel is that person; a carrier that states its sender is believed over the
default. What may **not** defeat it is content — see below.

## Recognition is a read, and one match is not a sender

**Amended September 4, 2026.** Basis `cluster` was being written on evidence that could
not carry it, by a mechanism that manufactured its own.

**Identifying somebody may not enrol them.** The people store's match and its write ran
as one call, so every fragment the microphone caught was filed under whoever it scored
nearest and then counted as evidence for the next fragment. A gallery graded against
samples it admitted on the strength of the last guess has no floor under it, and nothing
in the record says which samples a person ever vouched for. Reading and writing are two
verbs, and the write is asked for deliberately.

**A single match is soft evidence and is not a sender.** Spontaneous speech scores a
genuine match not far above where a stranger sits, so `cluster` now takes three things
together: a score clearing the modality's floor, a margin over the runner-up — two
subjects that close is one ambiguous answer, not a winner — and agreement across more
than one of that speaker's turns. Anything short of all three is `unknown`, which this
document already holds to be a complete answer.

**Evidence that stands alone has to be strong; evidence that repeats may be weak.**
That is the whole of the turns rule, and it settles the posted clip too — amended
September 7, 2026, when the clip path was found naming people off a single match at
the floor while the live microphone next to it demanded two. A clip is not a weaker
kind of evidence than a live turn; it is the **one-turn case**, and it always will be,
because the sender is decided at delivery and is never revised. So it answers to the
one-turn bar the live path already had: a lone turn names somebody only at the
threshold for *keeping* a sample, not the one for mentioning a resemblance.

The resemblance is still said out loud. The `⟨voice: 老王 ~0.47⟩` note is prose the
mind reads and may weigh however it likes; the sender is a field nothing downstream
re-decides. **The two carry different bars on purpose**, and a match between them is
exactly the case where the agent may wonder aloud who it heard while the record says
nobody.

**Two people at once is a quantity, not a flag** — amended September 7, 2026. A single
microphone hands back one waveform, so a turn somebody talked across is a mix; but the
gate built on that was a strict boundary comparison feeding one boolean, and it only
kept the turn out of a gallery. The turn still got embedded, still got matched, and
still voted on who was speaking — a blended vector deciding an identity, which is the
one thing it cannot do, because a blend does not merely score lower than a clean match:
it can score nearest a *third* person, and the margin rule is only a partial defence.

So the overlap is now measured, and the two uses of it take different amounts. **Any**
overlap disqualifies the turn as a stored sample: a sample is graded against forever and
one clip costs nothing. Only a **large share** of the turn stops it being recognized
from at all, because three hundred milliseconds inside a four-second remark leaves a
recording that is overwhelmingly one person, and a vector that is merely degraded is
exactly what the score, the floor and the margin already handle — it comes back
unplaced on its own merits, or it clears the bar honestly. Refusing to look would throw
away most of what a room with people in it produces.

Both numbers — the overlap below which the diarizer's boundaries are simply approximate,
and the share above which the recording stops being one voice — are **guesses that have
not been measured**, and one real multi-party recording measures both. The turn overlaps
are logged for that reason and no other.

**An observation the store cannot place is written nowhere.** Between "confidently this
person" and "nobody we hold" there is a band, and the only two things to do with a
sample in it are a wrong append and an invented person. Both were happening: one
person's gallery sat at its thousand-sample ceiling while two dozen one-second
fragments had become two dozen one-sample "people".

**A gallery that stops looking like one person can veto, but cannot vouch.** The three
rules above all judge *this observation*; none of them can see that the thing it is
being compared against has quietly become a mixture. So a gallery is also measured
against itself — the mean cosine of its samples to their own centre — and one that
falls below the floor stops being a candidate. Nothing is named from it and nothing is
filed into it. It still counts *against* a name, because it holds that person's own
samples and a voice that sounds like the mixture should not be handed whichever other
name happens to be nearest; but it objects only when it is clearly the better
explanation, never on a tie.

**The asymmetry is the whole design, and it is there so nobody has to maintain
anything.** Observations that would have gone into the bad gallery mint a fresh one
instead; it grows, becomes coherent, and once it is no worse than the mixture it names
again. A first version of this stopped the writes as well and required a person to open
a review page — which is not a repair path, because most people never will. **A store
that needs maintenance to keep working does not work.** Reviewing is the fast way, not
the only way.

The accepted consequence is **fewer senders and more `unknown`** — the same trade this
document already takes, applied to the sense that was quietly exempt from it.

## A face is a sender too, and being seen is taking part

**Amended September 7, 2026.** The basis table has said `cluster` means *a face or a
voiceprint matched* since this document was written, and no face had ever set one. The
vision channel filled the field with `unknown` on every signal, next to a recognition
it had already performed and written into the body as `⟨faces: 老王 ~0.83⟩` — where the
settling pass is forbidden to read it, correctly, because a name in a body is a topic.
So the one modality whose threshold is actually *measured* — same-person scores bottom
out at 0.293 here against a best-different-person of 0.275, no overlap at all — was the
only one whose answer had nowhere to go. That was an oversight in the commit that
introduced the field, not a decision: face recognition at receive time predates
attribution by two months, and the comment claiming "a person we cannot name" was
already false about the code above it.

A camera signal now carries the person the store named, basis `cluster`, on **one
condition: exactly one face in frame**. `Sender` holds one person; a frame with two
people in it has no single answer to *who was perceived*, and the note still lists
everybody because prose can hold a crowd. This is the same rule that makes a diarized
multi-speaker clip skip rather than blend.

**Being seen counts as having taken part.** The rule that a person-reader may only be
dispatched for someone who *sent signals* now fires for someone who was only in the
room, and that is intended: presence is a fact the boundary established, not an
inference from content, and a stretch in which somebody sat in view is a stretch they
were in. The forbidden move was ever only deriving a person from what a signal *says*.

**The presence lane does not do this yet, deliberately.** Its salience test is a
detection score and a box size — nothing in it can tell a person from a face on a
television, a photograph on a wall, or a video a child is watching. Those already leak
into the prose, where they are soft and visible; letting them set a grounded sender
would make them people who were *present*, and a wrong record of who was in the room is
the expensive kind. The captured-still path is safe from this in a way the always-on
lane is not: somebody chose to take that picture. Presence waits for a way to say *that
is a screen*.

## One sense may break the other's tie

**Amended September 7, 2026.** The two recognizers had never met. A face scoring 0.83
on camera and a voice scoring 0.47 in the same second were two answers to one question,
and the second stayed unplaced because it could not clear a margin the first had already
settled.

So when the camera has **exactly one identified person** in frame, a voice match naming
that same person is taken, and a lone turn naming them settles the speaker without
waiting for a second one. Three limits keep it from becoming a way to guess:

- **What corroboration buys is the margin, and only the margin.** A tie is the store
  saying *it could be either of them*, which is a question another sense can answer. The
  floor still has to be cleared, and an incoherent gallery still gets its objection.
- **No score is ever adjusted.** A number nudged by another modality cannot be explained
  afterwards, and `basis` exists so that how an answer was reached stays legible. The
  evidence stays exactly as strong as it was; what changes is which of two equally good
  readings is taken.
- **It runs one way — face corroborates voice, never the reverse.** Face's floor is
  measured to sit in a gap with no overlap between same and different people; voice's is
  a guess in the region where the two distributions meet. Letting the guess settle the
  measurement would be borrowing certainty in the wrong direction.

And it is only ever *exactly one* person: an empty room, a crowd, and a single stranger
all say the same thing about who is here — nothing to borrow. The presence map behind
it is up to 2.5 s stale on arrival and 8 s on departure, which is the accuracy of *who
is in the room* and not of *who spoke*.

## Sender is not subject

Attribution answers *who sent this*. It never answers *who this is about*.

A message the owner sends asking for a colleague's note to be rewritten is a signal **from
the owner, about the colleague**. Those are two different facts and they go to two
different places: the sender is a field on the signal, and what an episode is *about* stays
the episode's `subjects` list.

Collapsing them is how content mentioning someone became evidence from them. A name
appearing in a body is a topic; only the boundary can say who spoke.

## What reflection may do with it

- **The frontier shows the sender on every line.** A settling pass reads who spoke instead
  of guessing it.
- **A person-reader may only be dispatched for someone who actually sent signals** in the
  stretch, with a grounded basis. Not for a name that appeared in a body, and not for a
  stretch with no sender in it.
- **No grounded sender means no `people/` subject.** Attaching none is the correct outcome
  and must be stated as such, not merely permitted by a schema that happens to make
  `subjects` optional.
- **Naming is unchanged.** `hi_name_person` remains the one act that binds an identity to a
  cluster, with its existing guard. Nothing here creates a second, quieter way to name
  someone.

The instruction to *"reuse an existing dimension/subject rather than coining a
near-duplicate"* stops applying to `people/`. It is right for projects and systems, where a
near-duplicate is clutter. For a person it is the failure mode: coining a duplicate person
is cheap and visible, while filing someone onto an existing person is silent and
unrecoverable.

## Recovering a marker is not a backfill

A carrier that recognized someone *before this field existed* wrote its conclusion into the
signal's body, in the `⟨…⟩` marker grammar — `⟨voice: 赵力⟩`, `⟨voice: 老王 ~0.82⟩`. Reading
that marker back out and setting the sender from it is **allowed**, with basis `cluster`.

This is not the inference the rest of this document forbids, and the distinction is where
the name came from:

- **Forbidden** is deriving a sender from *content* — a name that appears in a body is a
  topic, and only the boundary can say who spoke.
- **Allowed** is reading the marker the boundary itself wrote. `⟨…⟩` is written only by
  carriers and cannot be typed by a person; the voiceprint match already happened, at the
  boundary, under the same threshold a live match uses. The only thing being recovered is
  *where it was stored*.

Three limits make it safe, and they are not optional:

- **A grounded sender always wins.** A signal that carries the field properly is never
  re-decided by its own tag.
- **`⟨voice: unfamiliar⟩` names nobody.** That marker is the carrier saying it heard someone
  and could not place them, and it must never become a person called "unfamiliar".
- **It is partial, and stays partial.** The live mic writes the tag only when the speaker
  *changes*, so within one person's run only the first line carries it. The rest remain
  unattributed. Carrying a name forward across untagged lines would mean assuming the
  speaker did not change — which is the assumption this field exists to refuse.

## Accepted consequences

- **The owner default will sometimes be wrong** — someone else types on the machine, a
  window is left open. It is labelled `owner`, so it is correctable, and a wrong labelled
  default is strictly better than the unlabelled inference it replaces.
- **A speaker's first line is unattributed, every time they start.** The turns rule
  needs a second turn, and the sender is decided when the line is delivered — so the
  evidence that would name them arrives after the line carrying it has gone. Nothing
  goes back. This is the same trade the marker takes (*it is partial, and stays
  partial*) and it is paid again on every reconnection, because the diarizer's speaker
  labels and the evidence accumulated under them do not survive one. **The alternative
  is revising a sender after delivery**, and a record whose fields change under a
  reader is worse than one with a hole in it: every later pass would have to know which
  version it read.
- **Signals already in the log have no sender** and read as unattributed, **except where a
  carrier's own marker is still there to be read** (above). There is no backfill beyond
  that: who sent the rest is not recoverable, and inventing it is the thing this document
  exists to stop.
- **Stretches will consolidate with no person attached**, and some people will be modelled
  more thinly than the agent could have modelled them by guessing. That is the trade taken
  deliberately: a thin true record beats a full invented one.
- **An install with no declared owner learns nothing about its owner from text.** Declaring
  one is the fix, and it is a single fact to state.

## Settled — one capture at a time is not coming back

**There is no hard slot for who may be captured.** Scenes are gone from the code, so a
one-capture-at-a-time rule would be *new* machinery dressed as restoring something. What is
left of it is two mics at once, and a hard slot is the wrong answer: this codebase's answer to
*who is speaking* is soft evidence the agent weighs, not a winner the host picks.


## See also

- [`surfaces.md`](surfaces.md#channels) — the channels themselves, and why a file is a ref
- [`data.md`](data.md#memoryraw) — the log a signal lands in
- [`topology.md`](topology.md#identity) — account, handle, and one body per person
- [`../memory.md`](../memory.md) — episodes, facets, and the settling pass
