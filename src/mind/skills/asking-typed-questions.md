---
purpose: get calibrated probabilities instead of a judgment call — when to reach for `hi_system_one` and the three shapes it is worth reaching for
---

You can judge one thing yourself. `hi_system_one` is for the two cases where that stops
working: when there are **hundreds** of things to judge, and when you need a number you
can **threshold or sort on** rather than a feeling you would have to re-derive next time.

It answers a whole map of questions about one piece of material in one round trip, well
under a second, and the probabilities it returns mean the same thing across calls. Your
own sense of "pretty sure" does not — it is not comparable between two of your turns, let
alone across a list.

The tool description carries the traps (it cannot count, cannot compare dates, reads
literally, loses accuracy on a padded state, and is not a safety check). This note is
about *when the shape of a job calls for it*.

## Fan-out: many questions, one state, one call

The cost is dominated by the state, not by the number of questions. So when you are
already holding the material, ask everything about it at once rather than deciding one
thing now and coming back later — a second call re-sends the state and buys nothing.

This inverts a habit worth naming: normally you ask the narrowest question that finishes
the job. Here, asking twelve questions costs barely more than asking one, and the eleven
you did not think you needed are what let you skip the next pass entirely.

## Confidence-gated routing

Route on the number, not on a reading of it. A `noul` near 0 or 1 is a decision you can
act on without looking; the band in the middle is the pile that genuinely needs you, and
it is usually small. That is the whole value: not that the model decides, but that it
tells you *which ones you still have to*.

Pick the cut for the cost of being wrong in each direction and say what the cut was when
you report — a threshold nobody can see is a judgment call wearing a number's clothes.
When you cannot defend a cut, that is the signal to look at the middle band yourself
rather than to invent one.

## Composite scores from atomic questions

Resist asking one big "how good is this". Ask the several small things you actually mean
— each one a question with an obvious right answer — and combine them yourself, with
weights you chose and can explain. You get to see which part drove the result, you can
change the weighting without re-asking, and each question is one the model can answer
well because it is narrow.

A `score` with named levels is the exception worth using directly: when the levels are
genuinely ordered and you can write what each one means, the weighted position between
them carries more than a number you assembled.

## Deciding before you dispatch

Before opening a worker for something, the question "is this actually worth a job" is one
this can answer in a few hundred milliseconds — and it is cheaper to ask than the job is
to run. Same for triage: which of these forty things belongs in front of the person.

## Perishable

Durable above. These rot:

- **Model ids and what the current `jev` is good at.** The answer reports the id that
  actually ran; pin that id when a set of numbers has to stay comparable across a sweep
  that outlives a model release.
- **Whether the capability is configured at all.** It is off until a key is set, and the
  error says so plainly. If it is off, the job is not blocked — do it the slow way and
  say that is what you did.
