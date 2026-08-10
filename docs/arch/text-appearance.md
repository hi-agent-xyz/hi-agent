# Text appearance

**Status:** accepted, intentionally breaking as of August 9, 2026.

## Decision

hi-agent has one backend-owned text appearance, regardless of how many windows are
open. It is current state, not a message log, mailbox, delivery queue, or transcript:

```json
{
  "user": "latest settled human line",
  "agent": { "text": "current accumulated reply", "final": false },
  "interim": "rolling speech-recognition partial"
}
```

Every field is optional. The state starts empty when the process starts.

`GET /api/out/text` is one long-lived NDJSON response. Its first line is the whole
current state; each later line replaces that state wholesale. The wire has no message
id, client id, cursor, acknowledgement, read position, or historical catch-up.

The journal is durable conversation history. The text appearance is only what the face
shows now.

## Ownership

The backend is the only authority on this state. A window never:

- invents an identity or partition;
- tells the backend what it has read;
- advances or consumes shared text;
- keeps a private reply queue;
- reconstructs the state from local storage.

Opening, foregrounding, refreshing, or reconnecting a window means "show me the present
state, then keep me synchronized." It does not mean "deliver everything I missed."

## State transitions

| Event | Authoritative result |
|---|---|
| Process starts | Empty text appearance |
| Rolling STT partial | Replaces `interim`; the settled exchange remains underneath |
| No newer partial for 3 seconds | Backend clears stale `interim` |
| Settled human line | Replaces `user`, clears `agent` and `interim`, and waits for a later reaction turn |
| Reaction turn starts | No visible change; output from this turn becomes eligible |
| First eligible agent chunk | Starts `agent`; keeps a waiting `user`, or clears an old `user` for an unsolicited expression |
| Later chunk in the same turn | Appends to `agent.text` and sets `final:false` |
| Utterance boundary | Sets `agent.final:true`; a later utterance in the same reaction turn may append and set it false again |
| Surface disconnects | No state change |
| Process restarts | State is empty; nothing is restored from the journal |

Eligibility is what keeps the current exchange honest. If a settled human line arrives
after a reaction turn started, that older turn may continue internally but it cannot
write more text onto the appearance. The next reaction turn can answer the new line.
This rule needs no identity on the wire; it is an internal ordering boundary.

## Interruption

Two different events had previously been called "interrupted outbound text."

### Transport interruption

A window can lose the HTTP response after seeing only part of a reply. That is not a
partially delivered message and there is nothing to resume. The backend still owns the
whole current reply. On reconnect, the window receives that whole state and replaces
its rendering.

No message id, cursor, acknowledgement, or per-window progress is needed because the
contract is convergence on the present, not exact delivery of intermediate chunks.

### Human interruption

A settled new human line becomes the current appearance immediately. Text still emitted
by the older reaction turn is not allowed to reclaim that line or appear to answer it.
The reaction itself is not cancelled: fix-forward and journal durability remain
unchanged. This rule only controls the text projection. Voice keeps its separate
barge-in and presence semantics.

## Multiplicity

Many windows render the same state. A slow window may skip intermediate growing-text
snapshots, but every live or reconnected window converges on the same latest snapshot.

Inbound audio and vision still accept distinct source connections and use internal
source/turn values where the carrier must prevent media bytes from interleaving in one
observer response. Those values are transport framing, not person, conversation, window,
or text-appearance identity, and they never appear in `/api/out/text`.

The text appearance intentionally has one rolling `interim` slot. If several speech
recognizers update it simultaneously, the latest update is what the face shows; their
settled signals remain durable in the journal. Supporting several simultaneous visible
transcripts would be a different product decision and would require source semantics,
not a client id disguised as one.

## Breaking boundary

There is no migration or compatibility path for the retired text-delivery protocol:

- old one-utterance plain-text long polls are unsupported;
- `epoch` and `after` query parameters have no meaning;
- `X-HI-Text-Epoch` and `X-HI-Utterance` are gone;
- the browser's old `hi-agent.out-text-cursor` value is ignored;
- old exchanges are not loaded into the new appearance.

Old clients and previous appearance data are allowed to break. This is deliberate.

## Accepted consequences

- There is no replay of an exchange after a newer one replaces it.
- Text is not restored after process restart.
- Slow surfaces may miss intermediate typing states.
- There is no read receipt or proof that a person saw the state.
- Per-window paced sentence reveal was removed because it made windows diverge. All
  windows now derive the same visible complete sentences from the same snapshot.
- One shared interim means simultaneous live transcripts are not separately displayed.

None of these lose durable conversation history; that remains the journal's job.
