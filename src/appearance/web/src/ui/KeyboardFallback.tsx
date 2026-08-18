import { useEffect, useRef, useState } from "react";

import { TYPING_PING_INTERVAL_MS, postInTextTyping } from "../channels/in/text";
import { isEditableTarget } from "../lib/handoff";

interface KeyboardFallbackProps {
  onSend: (text: string) => void;
  /** Whether the text channel is on (input line shown). Persisted by the hook. */
  open: boolean;
  /** Text pasted while the channel is open but focus is outside the input. */
  pastedText?: { id: number; text: string } | null;
  /** Turn the channel on — e.g. the user started typing while it was off. */
  onOpen: () => void;
  /** Turn the channel off — e.g. Esc. */
  onClose: () => void;
  /** Where the line sits: centred in the bottom band, or under the conversation
   * popover as its foot. The input follows the conversation, because typing into a
   * line nowhere near the messages is what makes a chat feel like a command bar. */
  anchor: "center" | "popover";
}

/**
 * The text input channel. When on, a minimal single line is shown that posts to
 * /thought; sending leaves it open (it's a channel, not a one-shot). When off,
 * the interface stays clean — but pressing any printable key turns it on and
 * seeds the first character, so a keyboard user never has to reach for a button.
 * Independent of the audio channels: usable with the mic on, off, or unavailable.
 */
export function KeyboardFallback({
  onSend,
  open,
  pastedText,
  onOpen,
  onClose,
  anchor,
}: KeyboardFallbackProps) {
  const [text, setText] = useState("");
  const inputRef = useRef<HTMLInputElement | null>(null);
  const lastPasteIdRef = useRef(0);
  const lastTypingPingRef = useRef(0);

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

  // Start-typing-to-open: a single printable key turns the channel on and seeds
  // the line. Only active while the channel is off.
  useEffect(() => {
    if (open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      // Someone else already owns the keystroke — a view's own input, say.
      if (isEditableTarget(e.target)) return;
      if (e.key.length === 1 && /\S/.test(e.key)) {
        // Swallow it. Opening the channel focuses the input inside this same
        // keydown, so the browser's default insertion would land in the input
        // we just seeded and type the character twice ("h" → "hh").
        e.preventDefault();
        setText(e.key);
        // The first character of a line counts as typing too — this is the path
        // that opens the channel, so it is where most drafts actually start.
        noteTyping();
        onOpen();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onOpen]);

  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open]);

  useEffect(() => {
    if (!open || !pastedText || pastedText.id === lastPasteIdRef.current) return;
    lastPasteIdRef.current = pastedText.id;
    setText((prev) => prev + pastedText.text);
    inputRef.current?.focus();
  }, [open, pastedText]);

  const submit = () => {
    const trimmed = text.trim();
    if (trimmed) onSend(trimmed);
    setText(""); // clear, but keep the channel open
  };

  if (!open) return null;

  return (
    <div className={anchor === "popover" ? "hi-kbd hi-kbd--popover" : "hi-kbd"}>
      <input
        ref={inputRef}
        data-hi-base-text-input
        value={text}
        spellCheck={false}
        onChange={(e) => {
          setText(e.target.value);
          // Emptying the line is not writing one — backspacing to nothing should
          // hand the floor straight back rather than hold it for the full window.
          if (e.target.value.trim()) noteTyping();
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            submit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            setText("");
            onClose();
          }
        }}
        placeholder="type to the agent…"
        aria-label="message the agent"
      />
    </div>
  );
}
