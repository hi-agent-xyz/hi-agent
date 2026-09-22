import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

/**
 * **One way to draw an attachment, wherever it is put** (`docs/arch/showing.md` §
 * *Presentation*): a tile on Home, a line in a task's panel, a bubble in the conversation, the
 * stage. They are here, in the face and in `@hi/core`, so a picture looks and behaves the same
 * in all of them and improving the player improves it everywhere — and so a view that embeds a
 * picture uses the same component the host draws it with, rather than a `fetch → blob → <video>`
 * of its own.
 *
 * What the face is handed is the server's description of one thing placed: an attachment
 * (`att:<id>`, with its preview and bytes) or a view a task made (`view:<ref>`, with its shot).
 */
export interface AttachmentItem {
  /** `att:<id>` or `view:<ref>`. */
  ref: string;
  /** `picture`, `clip` or `view`. */
  kind: string;
  /** The tile's picture. Absent while there is none yet. */
  preview?: string | null;
  /** An attachment's own bytes. A view has none: it is opened by its ref. */
  url?: string | null;
  label?: string | null;
  width?: number | null;
  height?: number | null;
  durationMs?: number | null;
}

const PREFIX = "att:";

/**
 * The item a conversation's file stands for. A file the agent handed over carries an `att:`
 * ref, whose bytes and preview are at the attachment route; one the person handed over is a
 * signal, served by `/api/media` as it always was.
 */
export function attachmentOf(ref: string, mime: string): AttachmentItem {
  const kind = mime.startsWith("image/") ? "picture" : mime.startsWith("video/") ? "clip" : "file";
  if (ref.startsWith(PREFIX)) {
    const id = ref.slice(PREFIX.length);
    return { ref, kind, url: `/api/attachments/${id}`, preview: `/api/attachments/${id}/preview.v1` };
  }
  return { ref, kind, url: `/api/media/${ref}`, preview: kind === "picture" ? `/api/media/${ref}` : null };
}

/** `0:30`, `12:04`, `1:02:09`. */
export function clock(ms: number): string {
  const s = Math.round(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor(s / 60) % 60;
  const r = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${r}` : `${m}:${r}`;
}

/**
 * What a tile draws inside the box its caller gives it: the preview, fitted whole — a figure is
 * whatever shape the work made it — or, while there is none or it will not load, what it is in
 * words. A clip wears its length either way. A view's shot is taken at the tile's own 16:9 and
 * fills the box instead.
 */
export function AttachmentPreview({ item, title }: { item: AttachmentItem; title: string }) {
  const [failed, setFailed] = useState(false);
  const length = item.kind === "clip" && item.durationMs != null ? clock(item.durationMs) : null;
  return (
    <span className="relative flex size-full items-center justify-center overflow-hidden">
      {item.preview && !failed ? (
        <img
          src={item.preview}
          alt={title}
          loading="lazy"
          onError={() => setFailed(true)}
          className={
            item.kind === "view"
              ? "block size-full object-cover object-top"
              : "block size-full object-contain"
          }
        />
      ) : (
        <span className="p-2 text-center text-xs font-semibold text-muted-foreground">
          {length ? `${title} · ${length}` : title}
        </span>
      )}
      {length && item.preview && !failed && (
        <span className="absolute right-1.5 bottom-1.5 rounded bg-black/60 px-1.5 py-px text-[11px] font-bold tabular-nums text-white">
          {length}
        </span>
      )}
    </span>
  );
}

/** The thing itself: a picture fitted whole, or a clip that plays and seeks. */
function Whole({ item, caption, autoPlay }: { item: AttachmentItem; caption?: string | null; autoPlay: boolean }) {
  if (item.kind === "clip" && item.url) {
    // `playsInline` for the reason the conversation's own videos carry it: without it an iPhone
    // takes a playing video to its own fullscreen player.
    return (
      <video
        src={item.url}
        poster={item.preview ?? undefined}
        controls
        autoPlay={autoPlay}
        playsInline
        preload="metadata"
        className="block max-h-full max-w-full rounded-md bg-black object-contain"
      />
    );
  }
  return (
    <img
      src={item.url ?? item.preview ?? ""}
      alt={caption ?? ""}
      className="block max-h-full max-w-full rounded-md object-contain"
    />
  );
}

/**
 * One attachment whole, over the whole window, with the words it was handed over with
 * underneath. Escape and a press outside close it; its Escape is taken before anything behind
 * it, so one key does not close the viewer and the panel it was opened from. Drawn into the
 * document's body, because it is opened from places — a panel, the conversation — whose own
 * boxes are too small to hold it and may be transformed.
 */
export function AttachmentViewer({
  item,
  caption,
  onClose,
}: {
  item: AttachmentItem;
  caption?: string | null;
  onClose: () => void;
}) {
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      close.current();
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, []);
  return createPortal(
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-[100] grid cursor-zoom-out place-items-center bg-black/80 p-6"
      onClick={(event) => {
        event.stopPropagation();
        onClose();
      }}
    >
      <figure
        className="m-0 flex max-h-full max-w-full cursor-default flex-col items-center gap-3"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex max-h-[78vh] max-w-full items-center justify-center">
          <Whole item={item} caption={caption} autoPlay />
        </div>
        {caption && (
          <figcaption className="max-w-[72ch] text-center text-[13px] leading-relaxed text-white/90">
            {caption}
          </figcaption>
        )}
      </figure>
      <button
        type="button"
        aria-label="Close"
        className="absolute top-3 right-4 size-9 rounded-full bg-white/15 text-2xl leading-none text-white"
        onClick={onClose}
      >
        ×
      </button>
    </div>,
    document.body,
  );
}

/**
 * An attachment on the stage: the frame it is handed, filled with the thing, fitted whole on the
 * room's own dark. What Reaction says beside it is the caption, so there is none here. The host
 * serves the module that mounts this (`/api/attachments/<id>/stage.v1.mjs`), so putting a picture
 * on the screen compiles nothing and needs no builder.
 */
export function AttachmentStage({ item }: { item: AttachmentItem }) {
  return (
    <div className="flex size-full items-center justify-center bg-black p-[max(16px,2vw)]">
      <Whole item={item} autoPlay={false} />
    </div>
  );
}
