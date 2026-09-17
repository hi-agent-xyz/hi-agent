import {
  useCallback,
  useEffect,
  useState,
  type DragEvent as ReactDragEvent,
} from "react";

import { postInFiles } from "../channels/in/file";
import {
  filesFromTransfer,
  isBaseTextInputTarget,
  isEditableTarget,
  transferHasFiles,
} from "../lib/handoff";

interface UseHandoffOptions {
  /** Bring the conversation up on the messages tab; a no-op where it already is. */
  openConversation: () => void;
  /** Append text to the line being written, with the caret after it. */
  pasteIntoTextInput: (text: string) => void;
}

/**
 * A file dropped or pasted anywhere on the face, and text pasted where nothing else
 * takes it.
 *
 * **The conversation is the account of a handover, so a handover opens it.** A file
 * that landed is a message in the list (`docs/arch/text-transcript.md`), drawn as
 * the thing that was sent (`ui/Chat.tsx`); pasted text goes into the line, where it
 * can be read before it is sent. There used to be a whole-face overlay here — "Drop
 * to send", "Sending 1 file", "Sent 1 file", a Retry — that blurred everything to
 * say, a moment later and less precisely, what the list says by growing a row.
 *
 * A batch that fails is logged and not drawn: what did not land is absent from the
 * list, and the gesture is there to make again. Batches are independent posts, so a
 * second drop during a first is sent rather than refused.
 */
export function useHandoff({ openConversation, pasteIntoTextInput }: UseHandoffOptions) {
  // How many batches are on the wire. Drawn by the picker (`ui/Composer.tsx`), so a
  // big file is not silent in the conversation until it lands.
  const [inFlight, setInFlight] = useState(0);

  const sendFiles = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return;
      openConversation();
      setInFlight((n) => n + 1);
      try {
        await postInFiles({ files });
      } catch (error) {
        console.error("files were not sent", error);
      } finally {
        setInFlight((n) => n - 1);
      }
    },
    [openConversation],
  );

  // Bound to enter as well as over: cancelling both is what makes the face a drop
  // target at all, and left alone the browser navigates to the dropped file.
  const onFileDragOver = useCallback((event: ReactDragEvent<HTMLDivElement>) => {
    if (!transferHasFiles(event.dataTransfer)) return;
    event.preventDefault();
    event.stopPropagation();
    event.dataTransfer.dropEffect = "copy";
  }, []);

  const onFileDrop = useCallback(
    (event: ReactDragEvent<HTMLDivElement>) => {
      if (!transferHasFiles(event.dataTransfer)) return;
      event.preventDefault();
      event.stopPropagation();
      void sendFiles(filesFromTransfer(event.dataTransfer));
    },
    [sendFiles],
  );

  const onClipboardPaste = useCallback(
    (event: ClipboardEvent) => {
      const data = event.clipboardData;
      if (event.defaultPrevented || data === null) return;

      const editable = isEditableTarget(event.target);
      const baseTextInput = isBaseTextInputTarget(event.target);
      if (editable && !baseTextInput) return;

      const files = filesFromTransfer(data);
      if (files.length > 0) {
        event.preventDefault();
        event.stopPropagation();
        void sendFiles(files);
        return;
      }

      // Let the host input perform an ordinary text paste at the caret.
      if (editable) return;

      const text = data.getData("text/plain");
      if (!text.trim()) return;
      event.preventDefault();
      event.stopPropagation();
      // What typing a character at the room does: the conversation comes up and the
      // words wait in the line. The line holds them until it is on screen
      // (`ui/Composer.tsx`), so this is the same whether or not it already was.
      openConversation();
      pasteIntoTextInput(text);
    },
    [openConversation, pasteIntoTextInput, sendFiles],
  );

  useEffect(() => {
    document.addEventListener("paste", onClipboardPaste, true);
    return () => document.removeEventListener("paste", onClipboardPaste, true);
  }, [onClipboardPaste]);

  // `sendFiles` is handed back because a picker calls it: the drop and the paste
  // are gestures a touch device does not have, so the line being written carries a
  // control that opens the system picker and hands what comes out to this same
  // path (`ui/Composer.tsx`).
  return {
    sending: inFlight > 0,
    sendFiles,
    onFileDragOver,
    onFileDrop,
  };
}
