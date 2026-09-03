import { useEffect, useRef, useState } from "react";
import { ArrowUpIcon } from "lucide-react";

import { TYPING_PING_INTERVAL_MS, postInTextTyping } from "../channels/in/text";
import { isEditableTarget } from "../lib/handoff";
import { onHostKey } from "../lib/keyboard";
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupTextarea,
} from "./shadcn/input-group";

interface ComposerProps {
  /** Rejects if the line never reached the server; the draft comes back if so. */
  onSend: (text: string) => Promise<void>;
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
 * **Any printable key puts the caret in the line and seeds it** — opening the
 * conversation first if it was away — so a keyboard user never has to reach for
 * the control, and, now that the same control puts the whole conversation away,
 * this is also the way back from having done so. That key, and a paste, are the
 * *only* things that move the caret here: **the conversation coming up does not
 * focus the line.** See the focus effect.
 */
export function Composer({ onSend, shown, pastedText, onOpen }: ComposerProps) {
  const [text, setText] = useState("");
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const lastPasteIdRef = useRef(0);
  const lastTypingPingRef = useRef(0);
  // Whether the conversation was already up a render ago, so the focus effect can
  // tell it coming up from it merely being up.
  const wasShownRef = useRef(shown);
  // Somebody asked for the caret while the conversation was still down — they
  // typed a character at the room. Held across the opening because the line is
  // mounted the whole time but not on screen yet, and focusing a hidden field
  // does nothing. Nothing else sets it: a conversation that simply comes up
  // arrives with no caret in it.
  const wantsCaretRef = useRef(false);

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

  // Start-typing: a single printable key puts the caret in the line and seeds it,
  // bringing the conversation back first if it was away. **This is now the whole
  // reason the caret ever moves on its own** — see the focus effect below — so it
  // runs in both states rather than only while the conversation is down.
  //
  // Through `onHostKey`, so it fires for the room and for chrome's own controls
  // but never while a view holds the focus — a view that binds a letter to page
  // itself must not also seed a message with it. Space and the arrows are not
  // printable by this test (`\S`, and a name longer than one character), so a
  // deck keeps them whenever the person is not typing into chrome.
  useEffect(() => {
    if (!shown && !onOpen) return;
    return onHostKey((e) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      // Someone else already owns the keystroke — the line itself once the caret
      // is in it, or a view's own input.
      if (isEditableTarget(e.target)) return;
      if (e.key.length === 1 && /\S/.test(e.key)) {
        // Swallow it. Focus lands in the line inside this same keydown (or on the
        // next frame, when the conversation has to come up first), so the
        // browser's default insertion would land in the field we just seeded and
        // type the character twice ("h" → "hh"). It is also the host's claim on
        // the key, which is what keeps it from reaching the view underneath.
        e.preventDefault();
        // Appended, not assigned. The line keeps a half-written draft through a
        // put-away on purpose, and while the conversation is up the caret can
        // simply be elsewhere — a control in chrome — with a sentence already
        // sitting in the box. Either way the character is the next one in what is
        // being written, so overwriting the draft with it would throw away the
        // thing this component exists to hold on to.
        setText((prev) => prev + e.key);
        // The first character of a line counts as typing too — this is the path
        // that opens the conversation, so it is where most drafts actually start.
        noteTyping();
        if (shown) {
          inputRef.current?.focus();
        } else {
          // The line is mounted but inside a surface that is not up yet; focusing
          // it here would be focusing something hidden. Claim the caret and let
          // the effect below take it once the conversation is on screen.
          wantsCaretRef.current = true;
          onOpen?.();
        }
      }
    });
  }, [shown, onOpen]);

  // The caret follows the ask for the *line*, never the opening of the
  // conversation. Opening it used to focus here unconditionally, which on a phone
  // means the software keyboard rises over the conversation the person just
  // tapped to read — half the screen spent on a box they did not ask for. So the
  // only two paths that move the caret are the ones where writing is the point:
  // typing a printable key (above) and pasting (below). Tapping the line itself
  // is a third, and it needs no code.
  useEffect(() => {
    const was = wasShownRef.current;
    wasShownRef.current = shown;
    if (shown && !was && wantsCaretRef.current) inputRef.current?.focus();
    // Spent either way: a claim is for the one opening it was made during, and a
    // conversation put away and brought back is a new one to arrive with.
    wantsCaretRef.current = false;
  }, [shown]);

  useEffect(() => {
    if (!shown || !pastedText || pastedText.id === lastPasteIdRef.current) return;
    lastPasteIdRef.current = pastedText.id;
    setText((prev) => prev + pastedText.text);
    inputRef.current?.focus();
  }, [shown, pastedText]);

  // Clear on send, but **only keep it cleared if the line actually went**. The box
  // emptied the instant Enter was pressed and the send's rejection was thrown away, so
  // a message the server refused looked precisely like one it accepted — the words
  // gone, nothing said. Optimistic still, because a line that lags behind the keypress
  // is its own kind of wrong; the draft simply comes back if the optimism was
  // misplaced. If they have started writing the next thing by then, that draft wins and
  // the failure is left to the console — overwriting what somebody is mid-sentence on
  // would be a worse loss than the one being repaired.
  const submit = () => {
    const trimmed = text.trim();
    if (!trimmed) return;
    setText(""); // clear, but keep the channel open
    void onSend(trimmed).catch((error: unknown) => {
      console.error("the line was not sent", error);
      setText((current) => (current ? current : trimmed));
    });
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
