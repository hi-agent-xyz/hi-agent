import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type DragEvent as ReactDragEvent,
} from "react";

import {
  FileUploadError,
  postInFiles,
  type UploadResult,
} from "../channels/in/file";
import {
  filesFromTransfer,
  isBaseTextInputTarget,
  isEditableTarget,
  transferHasFiles,
} from "../lib/handoff";

export type HandoffState =
  | "hover"
  | "sending"
  | "sent"
  | "partial"
  | "error";
export type HandoffKind = "files" | "text";

export interface HandoffFeedback {
  state: HandoffState;
  kind: HandoffKind;
  message: string;
  retryable: boolean;
}

interface UseHandoffOptions {
  scene: string;
  textInputOpen: boolean;
  sendText: (text: string) => void;
  pasteIntoTextInput: (text: string) => void;
}

const SUCCESS_VISIBLE_MS = 2400;

function fileCountLabel(count: number): string {
  return count === 1 ? "1 file" : `${count} files`;
}

function acceptedMessage(result: UploadResult, total: number): string {
  const received = result.received || total;
  return `Sent ${fileCountLabel(received)}`;
}

function failedFiles(error: FileUploadError, files: File[]): File[] {
  const failed = error.result?.failed ?? [];
  if (failed.length === 0) return files;
  const indexes = new Set(failed.map((item) => item.index));
  const retry = files.filter((_file, index) => indexes.has(index));
  return retry.length > 0 ? retry : files;
}

function uploadFailureFeedback(
  error: unknown,
  files: File[],
): { feedback: HandoffFeedback; retry: File[] } {
  if (error instanceof FileUploadError) {
    const retry = failedFiles(error, files);
    const received = error.result?.received ?? 0;
    if (received > 0) {
      return {
        feedback: {
          state: "partial",
          kind: "files",
          message: `Sent ${received} of ${files.length} files`,
          retryable: true,
        },
        retry,
      };
    }
    return {
      feedback: {
        state: "error",
        kind: "files",
        message:
          error.status === 413
            ? "Files must total 50 MB or less"
            : "File upload failed",
        retryable: error.status !== 413,
      },
      retry,
    };
  }

  return {
    feedback: {
      state: "error",
      kind: "files",
      message: "File upload failed",
      retryable: true,
    },
    retry: files,
  };
}

export function useHandoff({
  scene,
  textInputOpen,
  sendText,
  pasteIntoTextInput,
}: UseHandoffOptions) {
  const [feedback, setFeedback] = useState<HandoffFeedback | null>(null);
  const dragDepthRef = useRef(0);
  const statusTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const uploadAbortRef = useRef<AbortController | null>(null);
  const sendingRef = useRef(false);
  const retryFilesRef = useRef<File[]>([]);

  const clearStatusTimer = useCallback(() => {
    if (statusTimerRef.current !== null) {
      clearTimeout(statusTimerRef.current);
      statusTimerRef.current = null;
    }
  }, []);

  const showTimedFeedback = useCallback(
    (next: HandoffFeedback) => {
      clearStatusTimer();
      setFeedback(next);
      statusTimerRef.current = setTimeout(() => {
        setFeedback(null);
        statusTimerRef.current = null;
      }, SUCCESS_VISIBLE_MS);
    },
    [clearStatusTimer],
  );

  const dismiss = useCallback(() => {
    clearStatusTimer();
    setFeedback(null);
  }, [clearStatusTimer]);

  const sendFiles = useCallback(
    async (incoming: File[]) => {
      if (incoming.length === 0 || sendingRef.current) return;
      const files = [...incoming];
      sendingRef.current = true;
      retryFilesRef.current = files;
      clearStatusTimer();
      setFeedback({
        state: "sending",
        kind: "files",
        message: `Sending ${fileCountLabel(files.length)}`,
        retryable: false,
      });

      const abort = new AbortController();
      uploadAbortRef.current = abort;
      try {
        const result = await postInFiles({ scene, files, signal: abort.signal });
        retryFilesRef.current = [];
        showTimedFeedback({
          state: "sent",
          kind: "files",
          message: acceptedMessage(result, files.length),
          retryable: false,
        });
      } catch (error) {
        if (abort.signal.aborted) return;
        const failed = uploadFailureFeedback(error, files);
        retryFilesRef.current = failed.retry;
        clearStatusTimer();
        setFeedback(failed.feedback);
      } finally {
        if (uploadAbortRef.current === abort) uploadAbortRef.current = null;
        sendingRef.current = false;
      }
    },
    [clearStatusTimer, scene, showTimedFeedback],
  );

  const retry = useCallback(() => {
    const files = retryFilesRef.current;
    if (files.length > 0) void sendFiles(files);
  }, [sendFiles]);

  const resetDragHover = useCallback(() => {
    dragDepthRef.current = 0;
    setFeedback((current) => (current?.state === "hover" ? null : current));
  }, []);

  const onFileDragEnter = useCallback(
    (event: ReactDragEvent<HTMLDivElement>) => {
      if (!transferHasFiles(event.dataTransfer)) return;
      event.preventDefault();
      event.stopPropagation();
      event.dataTransfer.dropEffect = "copy";
      dragDepthRef.current += 1;
      if (dragDepthRef.current !== 1 || sendingRef.current) return;
      clearStatusTimer();
      setFeedback({
        state: "hover",
        kind: "files",
        message: "Drop to send",
        retryable: false,
      });
    },
    [clearStatusTimer],
  );

  const onFileDragOver = useCallback(
    (event: ReactDragEvent<HTMLDivElement>) => {
      if (!transferHasFiles(event.dataTransfer)) return;
      event.preventDefault();
      event.stopPropagation();
      event.dataTransfer.dropEffect = sendingRef.current ? "none" : "copy";
    },
    [],
  );

  const onFileDragLeave = useCallback(
    (event: ReactDragEvent<HTMLDivElement>) => {
      if (dragDepthRef.current === 0 && !transferHasFiles(event.dataTransfer)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
      if (dragDepthRef.current === 0) resetDragHover();
    },
    [resetDragHover],
  );

  const onFileDrop = useCallback(
    (event: ReactDragEvent<HTMLDivElement>) => {
      if (!transferHasFiles(event.dataTransfer)) return;
      event.preventDefault();
      event.stopPropagation();
      dragDepthRef.current = 0;
      const files = filesFromTransfer(event.dataTransfer);
      if (files.length === 0) {
        resetDragHover();
        return;
      }
      void sendFiles(files);
    },
    [resetDragHover, sendFiles],
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

      const rawText = data.getData("text/plain");
      const text = rawText.trim();
      if (!text) return;
      event.preventDefault();
      event.stopPropagation();
      if (textInputOpen) {
        pasteIntoTextInput(rawText);
        return;
      }
      sendText(text);
      showTimedFeedback({
        state: "sent",
        kind: "text",
        message: "Sent clipboard text",
        retryable: false,
      });
    },
    [
      pasteIntoTextInput,
      sendFiles,
      sendText,
      showTimedFeedback,
      textInputOpen,
    ],
  );

  useEffect(() => {
    document.addEventListener("paste", onClipboardPaste, true);
    return () => document.removeEventListener("paste", onClipboardPaste, true);
  }, [onClipboardPaste]);

  useEffect(() => {
    document.addEventListener("dragend", resetDragHover);
    document.addEventListener("drop", resetDragHover);
    window.addEventListener("blur", resetDragHover);
    return () => {
      document.removeEventListener("dragend", resetDragHover);
      document.removeEventListener("drop", resetDragHover);
      window.removeEventListener("blur", resetDragHover);
    };
  }, [resetDragHover]);

  useEffect(() => {
    return () => {
      clearStatusTimer();
      uploadAbortRef.current?.abort();
    };
  }, [clearStatusTimer]);

  return {
    feedback,
    isSending: feedback?.state === "sending",
    sendFiles,
    retry,
    dismiss,
    onFileDragEnter,
    onFileDragOver,
    onFileDragLeave,
    onFileDrop,
  };
}
