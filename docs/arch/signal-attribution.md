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
- **Signals already in the log have no sender** and read as unattributed, **except where a
  carrier's own marker is still there to be read** (above). There is no backfill beyond
  that: who sent the rest is not recoverable, and inventing it is the thing this document
  exists to stop.
- **Stretches will consolidate with no person attached**, and some people will be modelled
  more thinly than the agent could have modelled them by guessing. That is the trade taken
  deliberately: a thin true record beats a full invented one.
- **An install with no declared owner learns nothing about its owner from text.** Declaring
  one is the fix, and it is a single fact to state.

## See also

- [`surfaces.md`](surfaces.md#channels) — the channels themselves, and why a file is a ref
- [`data.md`](data.md#memoryraw) — the log a signal lands in
- [`topology.md`](topology.md#identity) — account, handle, and one body per person
- [`../memory.md`](../memory.md) — episodes, facets, and the settling pass
