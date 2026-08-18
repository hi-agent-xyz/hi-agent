import { useEffect, useRef, useState } from "react";
import { ArrowUpIcon } from "lucide-react";

import { TYPING_PING_INTERVAL_MS, postInTextTyping } from "../channels/in/text";
import { isEditableTarget } from "../lib/handoff";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupTextarea,
} from "./shadcn/input-group";

interface ComposerProps {
  onSend: (text: string) => void;
  /** Whether the conversation — and so this line standing in its foot — is on
   * screen. Not a mount switch: the line stays mounted while the conversation is
   * away, because that is what keeps a half-written draft through a put-away. */
  shown: boolean;
  /** Text pasted while the conversation is up but focus is outside the line. */
  pastedText?: { id: number; text: string } | null;
  /** Bring the conversation back — the person started typing while it was away.
   * `null` where there is nothing to bring back: a view rendering the words
   * itself owns the writing of them too, and swallowing the keystroke to open a
   * surface that stays down would simply lose the character. */
  onOpen: (() => void) | null;
}

/**
 * The line the person writes on: **the foot of the conversation, not a surface of
 * its own.** It used to be a separately positioned box (`.hi-kbd`) that the
 * stylesheet held flush under the popover by sharing the panel's width and right
 * edge and pushing the panel's floor up a row to make space — two sets of numbers
 * kept in step by hand so that one box would read as part of another. Rendered
 * inside the panel it simply *is* part of it, and both sets are gone.
 *
 * It is a `shadcn` `InputGroup` — a textarea that grows with what is typed, with
 * the send button inside its own box. Enter sends; Shift+Enter breaks a line,
 * which the single-line `<input>` this replaced could not offer at all. Sending
 * leaves it open: it is a channel, not a one-shot.
 *
 * **Any printable key opens the conversation and seeds the line**, so a keyboard
 * user never has to reach for the control — and, now that the same control puts
 * the whole conversation away, this is also the way back from having done so.
 */
export function Composer({ onSend, shown, pastedText, onOpen }: ComposerProps) {
  const [text, setText] = useState("");
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const lastPasteIdRef = useRef(0);
  const lastTypingPingRef = useRef(0);
  // Whether the conversation was already up when this window opened. A window that
  // starts with it up must not also start with the caret in the line: on a phone
  // that opens the on-screen keyboard over the conversation the person came to
  // read. Focus is for the transition — they asked for the line just now.
  const wasShownRef = useRef(shown);

  // Report that a line is being written, so the agent waits for the thought
  // rather than answering the part of it that already landed. Throttled rather
  // than debounced: the server wants to hear *while* they type, and a trailing
  // debounce would report the draft only once it had already stopped moving.
  const noteTyping = () => {
    const now = Date.now();
    if (now - lastTypingPingRef.current < TYPING_PING_INTERVAL_MS) return;
    lastTypingPingRef.current = now;
    postInTextTyping();
  };

  // Start-typing-to-open: a single printable key brings the conversation back and
  // seeds the line. Only active while it is away.
  useEffect(() => {
    if (shown || !onOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      // Someone else already owns the keystroke — a view's own input, say.
      if (isEditableTarget(e.target)) return;
      if (e.key.length === 1 && /\S/.test(e.key)) {
        // Swallow it. Opening focuses the line inside this same keydown, so the
        // browser's default insertion would land in the field we just seeded and
        // type the character twice ("h" → "hh").
        e.preventDefault();
        setText(e.key);
        // The first character of a line counts as typing too — this is the path
        // that opens the conversation, so it is where most drafts actually start.
        noteTyping();
        onOpen();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [shown, onOpen]);

  useEffect(() => {
    const was = wasShownRef.current;
    wasShownRef.current = shown;
    if (shown && !was) inputRef.current?.focus();
  }, [shown]);

  useEffect(() => {
    if (!shown || !pastedText || pastedText.id === lastPasteIdRef.current) return;
    lastPasteIdRef.current = pastedText.id;
    setText((prev) => prev + pastedText.text);
    inputRef.current?.focus();
  }, [shown, pastedText]);

  const submit = () => {
    const trimmed = text.trim();
    if (trimmed) onSend(trimmed);
    setText(""); // clear, but keep the channel open
  };

  return (
    <InputGroup className="hi-composer">
      <InputGroupTextarea
        ref={inputRef}
        data-hi-base-text-input
        value={text}
        rows={1}
        spellCheck={false}
        onChange={(e) => {
          setText(e.target.value);
          // Emptying the line is not writing one — backspacing to nothing should
          // hand the floor straight back rather than hold it for the full window.
          if (e.target.value.trim()) noteTyping();
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            submit();
          } else if (e.key === "Escape" && text) {
            // Clear the draft, and stop there: the press is spent. An empty line
            // lets Escape through to the shell, which is what closes a popover —
            // so the ladder is "throw away what I typed", then "put it away".
            e.preventDefault();
            setText("");
          }
        }}
        placeholder="type to the agent…"
        aria-label="message the agent"
      />
      <InputGroupAddon align="inline-end">
        <InputGroupButton
          size="icon-xs"
          variant="ghost"
          disabled={!text.trim()}
          onClick={submit}
          title="send"
          aria-label="send"
        >
          <ArrowUpIcon />
        </InputGroupButton>
      </InputGroupAddon>
    </InputGroup>
  );
}
