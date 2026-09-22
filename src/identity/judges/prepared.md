# You read where the person's next message went — as typed questions

When Reaction has prepared branches (`hi_prepare`) and the person's next message lands, the host
asks System One two typed questions about it (`docs/arch/agents.md` § *Prepared branches*). This
page is the wording of those questions. Each `##` section below is sent verbatim for the part
its heading names; this paragraph and the next are for whoever edits the page and are never
sent.

One `choice` is asked: **Frame**, then **Which way**, over one option per branch — its criterion
is the branch's own condition, as Reaction wrote it — and one more option, `rest`, meaning
**Rest**. One `noul` is asked: **Frame**, then **Qualified**. A branch runs only when its option
carries most of the mass and `qualified` carries almost none, so a doubtful reading costs a
turn, never a wrong action.

## Frame
The state is a short stretch of a conversation: what an assistant said, then what the person said next. The assistant had guessed, before the person answered, a few directions their answer might take. Read only the person's part, literally, and in the light of what it answers.

## Which way
Which of these directions did the person's message actually take? Choose `rest` unless the message plainly goes the way one option describes.

## Rest
none of the directions above: a different view, a question, a correction, a change of subject, something unclear or ambiguous, or anything else not plainly described by another option

## Qualified
Does the person's message attach a condition, a reservation, a correction or a doubt to what it agrees to or chooses — anything that would change what should now be done about it? Going on to ask for a separate, further thing is not that; only something that changes the thing agreed to or chosen counts.
