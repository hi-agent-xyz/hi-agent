import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
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
    // A clip is played from the route that says where its playable bytes are — the original,
    // or the copy the host makes of one no browser decodes.
    const url = kind === "clip" ? `/api/attachments/${id}/playable` : `/api/attachments/${id}`;
    return { ref, kind, url, preview: `/api/attachments/${id}/preview.v1` };
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

/** How long a clip whose playable copy is still being made waits before asking again, and for
 * how long it keeps asking. The route answers `503` until the copy lands; a transcode of a long
 * clip can take minutes. */
const RETRY_MS = 3000;
const GIVE_UP_MS = 20 * 60_000;

/**
 * A clip that plays and seeks. When the route says its copy is still being made, the player
 * shows the frame it has and asks again — so the stage, the panel and the conversation need no
 * notion of *ready*: the one URL answers, eventually, with bytes a browser plays.
 */
export function AttachmentClip({
  item,
  autoPlay,
  fill = false,
}: {
  item: AttachmentItem;
  autoPlay: boolean;
  /** Take the width it is given, as a clip laid out inside a view does, rather than fitting
   * whole inside a box that is already sized. */
  fill?: boolean;
}) {
  const [attempt, setAttempt] = useState(0);
  const [waiting, setWaiting] = useState(false);
  const since = useRef(Date.now());
  useEffect(() => {
    if (!waiting) return;
    if (Date.now() - since.current > GIVE_UP_MS) return;
    const timer = setTimeout(() => {
      setWaiting(false);
      setAttempt((n) => n + 1);
    }, RETRY_MS);
    return () => clearTimeout(timer);
  }, [waiting]);
  return (
    <span className={fill ? "relative flex w-full" : "relative inline-flex max-h-full max-w-full"}>
      {/* `playsInline` for the reason the conversation's own videos carry it: without it an
          iPhone takes a playing video to its own fullscreen player. */}
      <video
        key={attempt}
        src={item.url ?? undefined}
        poster={item.preview ?? undefined}
        controls
        autoPlay={autoPlay}
        playsInline
        preload="metadata"
        onError={() => setWaiting(true)}
        width={fill ? (item.width ?? undefined) : undefined}
        height={fill ? (item.height ?? undefined) : undefined}
        className={
          fill
            ? "block h-auto w-full rounded-md bg-black object-contain"
            : "block max-h-full max-w-full rounded-md bg-black object-contain"
        }
      />
      {waiting && (
        <span className="pointer-events-none absolute inset-x-0 bottom-12 text-center text-xs font-semibold text-white/85">
          Making a copy that plays here…
        </span>
      )}
    </span>
  );
}

/** The thing itself: a picture fitted whole, or a clip that plays and seeks. */
function Whole({ item, caption, autoPlay }: { item: AttachmentItem; caption?: string | null; autoPlay: boolean }) {
  if (item.kind === "clip" && item.url) {
    return <AttachmentClip item={item} autoPlay={autoPlay} />;
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

/** What each attachment a page embeds is, asked once per id however many times it is drawn. The
 * answer is `immutable` at the route, so the browser keeps it too. */
const described = new Map<string, Promise<AttachmentItem | null>>();

function describe(id: string): Promise<AttachmentItem | null> {
  let known = described.get(id);
  if (!known) {
    known = fetch(`/api/attachments/${id}/about.v1`)
      .then((r) => (r.ok ? (r.json() as Promise<AttachmentItem>) : null))
      .catch(() => null);
    described.set(id, known);
  }
  return known;
}

/** The long side a `preview.v1` is fitted to — `docs/arch/showing.md` § *Derivations*. */
const PREVIEW_BOX = { width: 960, height: 540 };

/**
 * Which bytes a picture drawn `cssWidth` wide should load: the preview while it is sharp enough
 * at this screen's density, the original once the box is wider than the preview is. A page
 * that embeds twenty figures loads twenty tiles, not twenty originals; a picture laid across a
 * whole frame loads the whole picture.
 */
export function pictureSource(item: AttachmentItem, cssWidth: number, density: number): string {
  const original = item.url ?? item.preview ?? "";
  if (!item.preview || !item.width || !item.height) return original;
  const scale = Math.min(1, PREVIEW_BOX.width / item.width, PREVIEW_BOX.height / item.height);
  return cssWidth * density <= item.width * scale ? item.preview : original;
}

/**
 * An attachment inside a view: `<Attachment id="att:3f9a…" />`, drawn by the same components the
 * host draws it with everywhere else (`docs/arch/showing.md` § *Inside a view*). The view names
 * the id and nothing else — what the thing is, where its bytes are and whether a clip needs its
 * playable copy are the host's to say.
 *
 * A picture takes the width its box gives it at its own shape, and opens whole in the viewer
 * when pressed; a clip plays in place. The preview is drawn from the first frame, before the
 * host has said which it is, so a page is never laid out around an empty box. An id this core
 * does not hold is said in the page and reported as an error, so a review catches it before the
 * person does.
 */
export function Attachment({
  id,
  caption,
  className,
  style,
}: {
  /** `att:<id>`, as the host answered when it was placed. */
  id: string;
  /** What it shows, for the viewer and for anyone who cannot see it. */
  caption?: string;
  className?: string;
  style?: CSSProperties;
}) {
  const hex = id.trim().replace(/^att:/, "");
  const valid = /^[0-9a-f]{16}$/.test(hex);
  const [item, setItem] = useState<AttachmentItem | null | undefined>(undefined);
  const [open, setOpen] = useState(false);
  const [width, setWidth] = useState(0);
  const box = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!valid) {
      console.error(`<Attachment id="${id}"> — that is not an attachment id; use the att: id the host answered with`);
      setItem(null);
      return;
    }
    let alive = true;
    describe(hex).then((found) => {
      if (!alive) return;
      if (!found) console.error(`<Attachment id="${id}"> — no such attachment on this agent`);
      setItem(found);
    });
    return () => {
      alive = false;
    };
  }, [hex, valid, id]);

  // Only ever grows: a picture that has loaded its original never goes back to the tile.
  useEffect(() => {
    const el = box.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const seen = new ResizeObserver(([entry]) => {
      const w = entry?.contentRect.width ?? 0;
      setWidth((was) => (w > was ? w : was));
    });
    seen.observe(el);
    return () => seen.disconnect();
  }, []);

  const shape = item?.width && item?.height ? `${item.width} / ${item.height}` : undefined;
  const density = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;

  let body: ReactNode;
  if (item === null) {
    body = (
      <span className="flex aspect-video w-full items-center justify-center rounded-md bg-muted p-3 text-center text-xs text-muted-foreground">
        {valid ? `${id} is not on this agent` : `${id} is not an attachment id`}
      </span>
    );
  } else if (item?.kind === "clip") {
    body = <AttachmentClip item={item} autoPlay={false} fill />;
  } else {
    const src = item ? pictureSource(item, width, density) : `/api/attachments/${hex}/preview.v1`;
    body = (
      <img
        src={src}
        alt={caption ?? ""}
        width={item?.width ?? undefined}
        height={item?.height ?? undefined}
        onClick={item ? () => setOpen(true) : undefined}
        style={shape ? { aspectRatio: shape } : undefined}
        className="block h-auto w-full cursor-zoom-in rounded-md object-contain"
      />
    );
  }

  return (
    <figure
      ref={box}
      className={["m-0", className].filter(Boolean).join(" ")}
      style={style}
      data-attachment={valid ? `att:${hex}` : undefined}
    >
      {body}
      {open && item && <AttachmentViewer item={item} caption={caption} onClose={() => setOpen(false)} />}
    </figure>
  );
}
