# You read a stop in the conversation — as typed questions

Everything Reaction says or shows is prepared (`hi_prepare`) and held until the room reaches the
moment it was written for. At every stop — the person has stopped, for now — the host asks
System One typed questions about it (`docs/arch/host.md` § *The floor*, `docs/arch/agents.md`
§ *Prepared actions*). This page is the wording of those questions. Each `##` section below is
sent verbatim for the part its heading names; this paragraph and the next two are for whoever
edits the page and are never sent.

**The floor.** One `noul`: **Frame**, then **Finished** — asked unless the one wait ran out.
And one `choice` per branch that is waiting for a floor moment: **Frame**, then **Fits finished**
or **Fits paused**, then the matter and what is ready, over three options whose criteria are
**Say**, **Hold** and **Drop**. A `finished` set goes only when `finished` clears its cut and its
own choice is `say`; a `paused` one only when `finished` does not and its choice is `say`.

**Their message.** When the stop carries a message and a matter has condition branches that are
live (its answer has already gone), one `choice` is asked: **Frame**, then **Which way**, over one
option per condition of every live matter — its criterion is the matter's name, a dash, and the
condition as Reaction wrote it — and one more option, `rest`, meaning **Rest**. One `noul`:
**Frame**, then **Qualified**. And one `noul` per live matter: **Frame**, then **Took it up**, then
the matter's name. A branch runs only when its option carries most of the mass and `qualified`
carries almost none, so a doubtful reading costs a turn, never a wrong action.

## Frame
The state is part of a conversation between a person and an assistant. For each of a few matters it shows where the matter was left — the assistant's words on it — and, for a reply the assistant has ready but has not yet said, what the person said after that reply was written. Then what the assistant said most recently and how many messages it has sent since the person last wrote, then what the person said at this stop — or, if nothing, how long they have been quiet. Each of the person's lines says how long ago it was said. Read the person's part literally, in the light of what it answers: a short reply answers what was said most recently, not a matter left earlier.

## Finished
Has the person finished what they were saying for now — a question asked, a point made, an instruction given — so that a reply is what they are waiting for? Or have they stopped part-way, with more plainly coming: a sentence left open, "and also", "then", "还有", one question in a run of questions they are still asking, a list in the middle, a story not yet at its point? What they said is evidence, and so is when they said it — each line carries how long ago it came. Lines arriving seconds apart are a run still going, and a line in the middle of such a run is usually followed by another, even when it reads complete on its own. A line that comes after minutes of quiet, or after the assistant has answered, more often stands alone. A short pause alone is not finishing; a long quiet is: a line left minutes ago with nothing after it is finished, however it trails off.

## Fits finished
A reply the assistant prepared, to go out when the person finishes. Against what the person has said since it was written, which is right now?

## Fits paused
A short acknowledgment the assistant prepared, to go out if the person stops with more to come. At this stop, which is right now?

## Say
say it now: it still answers what the person has said and is what they are waiting for at this point — or, for an acknowledgment, it fits this pause naturally. An answer to something the person asked — just now or earlier, including the result of work they asked for — is owed, however many messages came before it: a run of questions is owed a run of answers. The count of messages already sent matters only for what nobody asked for — a progress note, an aside, a reminder, a remark about the screen: each one already sent since the person last wrote makes one more of those less worth reading now

## Hold
keep it for later: it is still correct, but this is not the moment — the person is in the middle of an exchange about something else and this would cut into it, or it is an aside that would interrupt what they are saying, or an acknowledgment that would break into their thought, or it is something nobody asked for, the assistant has already sent several since they last wrote, and it can wait until they next write. Being quiet or away is not being busy: a message waits in the conversation until they read it, so a quiet stretch is when an answer or a result they have not seen can go, not a reason to keep it

## Drop
discard it: what the person said since it was written makes it wrong or no longer wanted, or it is answered already — by the person, or by something the assistant said most recently that covers the same thing, even in other words

## Which way
Which of these directions did the person's message actually take? Choose `rest` unless the message plainly goes the way one option describes, about that option's matter.

## Rest
none of the directions above: a different view, a question, a correction, a change of subject, something about another matter, something unclear or ambiguous, or anything else not plainly described by another option

## Qualified
Does the person's message attach a condition, a reservation, a correction or a doubt to what it agrees to or chooses — anything that would change what should now be done about it? Going on to ask for a separate, further thing is not that; only something that changes the thing agreed to or chosen counts.

## Took it up
Does the person's message take up the matter named below — answer it, decide it, push it forward, question it or turn it somewhere — rather than being about something else? A message about a different subject, or too unclear to tell, does not take it up.
