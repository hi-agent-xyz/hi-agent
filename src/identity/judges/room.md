# Whether what the room just said is anyone talking with the agent

The room screen does not ask a model to write anything. When a batch of new signals is only
room — speech the microphone caught, a face the camera saw — the host asks System One one typed
question about it before waking Reaction, and a batch that reads as nobody talking with the
agent is set aside instead of waking it (`docs/arch/host.md` § *The room screen*). This page is
the wording. Each `##` section below is sent verbatim for the part its heading names; this
paragraph and the next are for whoever edits the page and are never sent.

The state is **Setting**, then the recent transcript with each line's age in seconds, then when
the agent last spoke, then the batch under *New lines just captured*. **Question** is one `noul`.
The wording is the one measured on 2026-09-21 against 650 labelled batches; a change to it is a
change to that measurement, and the threshold (`room_screen_wake_at`) was chosen against it.

## Setting
An AI assistant runs on a computer at home / in an office. Its microphone hears everything in the room through speech recognition, which makes errors and emits fragments. People sometimes talk with the assistant — asking it things, answering it, dictating thoughts for it to record, reviewing a page it made, playing a game it is part of — usually without naming it, and often talk to each other, to children, on the phone, or a video / car navigation is playing. Marks in ⟨…⟩ are the microphone's unreliable notes. '[heard]' = microphone, '[typed]' = typed to the assistant, '[assistant]' = what the assistant said, '[camera]' = presence event. Times are seconds before the new lines.

## Question
Is someone in the new lines talking WITH the assistant — addressing it, answering it, or continuing a conversation, dictation, review of its work, or a game it is part of — even without naming it?
