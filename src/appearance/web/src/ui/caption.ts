import { useEffect, useState } from "react";

/**
 * When the pill is up, and for how long.
 *
 * The pill is not the record any more — the conversation keeps
 * (`docs/arch/text-transcript.md`) and the popover is one press away — so a line
 * that has been read has no further claim on the frame it is sitting over. It
 * behaves like a caption again: it appears, it is read, it goes.
 *
 * This is not the old caption band's timer under a new name. That timer was
 * *spending* the words: the band revealed an utterance the buffer had already
 * deleted, so whatever it advanced past was gone (`arch-refactor.md`, the
 * half-spent-text finding). This one hides a copy that is still in the list
 * behind it. That difference is the whole argument — a timer over a durable list
 * costs nothing; a timer over a queue costs the message.
 */

/** Floor: even one word stays long enough to be noticed and read. */
const MIN_MS = 4_000;
/** Ceiling: the pill clamps to three lines, so past a point there is no more to read. */
const MAX_MS = 10_000;
/** The part that is not reading — noticing the line, and settling after it. */
const BASE_MS = 2_000;
/** Unhurried, and mixed script: a Chinese line carries more per character. */
const CHARS_PER_SECOND = 12;

/** How long a settled line of `text` holds before it fades. */
export function captionDwell(text: string): number {
  const reading = (text.length / CHARS_PER_SECOND) * 1_000;
  return Math.min(MAX_MS, Math.max(MIN_MS, BASE_MS + reading));
}

/** The tail of the conversation, as the pill needs it. */
export interface CaptionLine {
  /** When it was said — not when this window happened to see it. */
  ts: string;
  text: string;
}

/**
 * When a settled line stops being worth showing, in epoch ms — or `null` if it
 * never does.
 *
 * The clock runs from the line's own `ts`, not from when this window saw it, and
 * that one choice answers reload, resync and a second device joining mid-
 * conversation without a rule apiece: a line said an hour ago is already past its
 * dwell, so opening a window never flashes a spent sentence over the view.
 */
export function captionDeadline(line: CaptionLine | undefined): number | null {
  if (!line) return null;
  const spoken = Date.parse(line.ts);
  return Number.isNaN(spoken) ? null : spoken + captionDwell(line.text);
}

export interface CaptionInput {
  /** The live recognition partial. Held for as long as it rolls. */
  interim?: string | undefined;
  /** The settled line the conversation currently ends on. */
  line?: CaptionLine | undefined;
}

/**
 * Whether the pill is revealed right now. A rolling interim has no deadline at
 * all — it is still being said; a settled line has `captionDeadline`'s.
 *
 * Nothing here holds the fade off while the person is reaching for a link in the
 * line; that is the stylesheet's `:has(.hi-speech-link:hover)`, which cannot get
 * stuck the way a `held` flag does when the dock unmounts under the pointer.
 */
export function useCaption({ interim, line }: CaptionInput): boolean {
  // Starts expired so a stale line is never painted for a frame on its way to
  // being hidden. A fresh line reveals from the effect below, which it would have
  // faded into regardless.
  const [expired, setExpired] = useState(true);

  // `null` — nothing to count down: speech still rolling, nothing said yet, or a
  // timestamp we cannot read. The first two are covered by the guard on the
  // return; for the third, holding the line is the safer of the two failures.
  const deadline = interim ? null : captionDeadline(line);

  useEffect(() => {
    if (deadline === null) {
      setExpired(false);
      return;
    }
    const remaining = deadline - Date.now();
    if (remaining <= 0) {
      setExpired(true);
      return;
    }
    setExpired(false);
    const timer = setTimeout(() => setExpired(true), remaining);
    return () => clearTimeout(timer);
  }, [deadline]);

  return (!!interim || !!line) && !expired;
}
