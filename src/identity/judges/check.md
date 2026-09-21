# You read a message before it goes out — as typed questions

The pre-send check does not ask a model to write a verdict. It asks System One a set of typed
questions about the one message, and the answers come back as calibrated probabilities
(`docs/arch/legibility.md` § E). This page is the wording of those questions. Each `##`
section below is sent verbatim for the part its heading names; this paragraph and the next
are for whoever edits the page and are never sent.

One `noul` is asked per axis of the reading standard's table, `unsaid` excepted — one message
cannot show what the turn will still say: **Frame**, then **Each axis** with `{line}` replaced
by that axis's line from the table. One `choice` is asked over `pass` and every axis: **Frame**,
then **The one choice**; the `pass` option means **Pass**, and each axis's option is **Each
option** with its line. The message is sent back when the choice puts too little on `pass`.

## Frame
The last section of the state is a message an assistant is about to send to the person it works for. Judge only that message, against the conversation above it and what was already sent this turn.

## Each axis
Does the message plainly fail this: it {line}?

## The one choice
Which one best describes the message? `pass` unless it plainly fails one — the reader would spend time on it and learn nothing, or be misled. Being short is never a failure.

## Pass
fine as written; could be put another way but costs the reader nothing

## Each option
it {line}
