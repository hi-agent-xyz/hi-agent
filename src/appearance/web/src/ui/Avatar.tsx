import { useEffect, useState } from "react";
import { url } from "../lib/base";
import type { Sender } from "../channels/out/text";

/**
 * The face beside a message.
 *
 * There are four things this can honestly say, and they are the four states the
 * people store itself has — the avatar is a rendering of what the agent knows about
 * who is talking, not decoration bolted onto a chat:
 *
 * | What is shown | What it means |
 * |---|---|
 * | a face crop | this person, seen — the oldest crop in their gallery |
 * | a coloured disc with an initial | someone with a name the camera has never met |
 * | a coloured disc with a silhouette | someone we can tell apart but cannot name yet |
 * | a plain disc with a silhouette | someone spoke and nobody could say who |
 *
 * The colour is derived from the subject and nothing else, so an unnamed cluster
 * keeps the same one everywhere it appears — which is the whole content of a cluster
 * id, drawn. Naming that cluster in 认识的人 changes the disc into an initial, and
 * the first time the camera meets them it becomes their face.
 *
 * **Nothing here decides who anyone is.** The subject arrives on the message from
 * the boundary that recognized it (`docs/arch/signal-attribution.md`); this file
 * only draws it.
 */

const SIZE_CLASS = "size-7 shrink-0 self-start overflow-hidden rounded-full";

/** `/api/people/<subject>/avatar` — the one crop that stands for a person. */
function avatarUrl(subject: string): string {
  return url(`/api/people/${encodeURIComponent(subject)}/avatar`);
}

/**
 * Whether a subject reads as a name rather than a freshly-minted cluster id — 8
 * base-36 lowercase chars with at least one digit, mirroring `mint_id` and the
 * backend's own `looks_like_cluster_id` (`server/vision.rs`). An all-letters name
 * like "samantha" carries no digit, so it is not mistaken for an id.
 *
 * A display question, answered where the display is: showing `7j2wa4r8` as the
 * initial "7" would dress an opaque id up as somebody's name.
 */
export function readsAsName(subject: string): boolean {
  return !(
    subject.length === 8 &&
    /^[a-z0-9]+$/.test(subject) &&
    /[0-9]/.test(subject)
  );
}

/**
 * A stable hue per subject. Only has to be deterministic and spread out — two people
 * colliding costs nothing, since the colour is a hint beside a name, never the
 * identity itself.
 */
function hueOf(subject: string): number {
  let h = 0;
  for (const ch of subject) h = (h * 31 + ch.codePointAt(0)!) % 360;
  return h;
}

/**
 * Has this person got a face to show? Asked once per subject per window rather than
 * per message: a voice-only cluster has no crop, and rendering an `<img>` for every
 * group of theirs would put one 404 on the wire per group. A hit lands in the HTTP
 * cache, so the `<img>` that follows costs nothing.
 *
 * Re-asked after `PROBE_TTL_MS`, which matches the route's own `max-age`: meeting
 * someone for the first time should put their face in the conversation without a
 * reload, and 5 minutes is how stale the served crop is already allowed to be.
 */
const PROBE_TTL_MS = 5 * 60 * 1000;
const probes = new Map<string, { at: number; found: Promise<boolean> }>();

function hasFace(subject: string): Promise<boolean> {
  const now = Date.now();
  const cached = probes.get(subject);
  if (cached && now - cached.at < PROBE_TTL_MS) return cached.found;
  const found = fetch(avatarUrl(subject), { cache: "default" })
    .then((res) => res.ok)
    .catch(() => false);
  probes.set(subject, { at: now, found });
  return found;
}

function useFace(subject: string | undefined): boolean {
  const [found, setFound] = useState(false);
  useEffect(() => {
    if (!subject) {
      setFound(false);
      return;
    }
    let live = true;
    void hasFace(subject).then((ok) => {
      if (live) setFound(ok);
    });
    return () => {
      live = false;
    };
  }, [subject]);
  return found;
}

/** Someone, unidentified — the shape a person makes when you cannot name them. */
function Silhouette() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden className="size-4 opacity-60" fill="currentColor">
      <circle cx="12" cy="8" r="4" />
      <path d="M4 21c0-4.4 3.6-7 8-7s8 2.6 8 7z" />
    </svg>
  );
}

/**
 * The agent's own mark. It is the one identity in this conversation that is not a
 * person in the store, so it is drawn from the app's mark rather than looked up.
 */
function AgentMark() {
  return (
    <div
      className={`${SIZE_CLASS} flex items-center justify-center bg-secondary`}
      aria-hidden
    >
      <img src={url("/icon.svg")} alt="" className="size-4 object-contain" />
    </div>
  );
}

export interface SenderAvatarProps {
  /** Absent for the agent's own messages. */
  sender?: Sender | undefined;
  role: "user" | "agent";
}

export function SenderAvatar({ sender, role }: SenderAvatarProps) {
  const subject = sender?.subject;
  const face = useFace(subject);

  if (role === "agent") return <AgentMark />;

  if (!subject) {
    return (
      <div
        className={`${SIZE_CLASS} flex items-center justify-center bg-muted text-muted-foreground`}
        title="someone — not recognized"
      >
        <Silhouette />
      </div>
    );
  }

  // The name, and how it was arrived at. A default the agent may be wrong about
  // reads as one, in the one place a person can see it and say so.
  const label = sender.basis === "owner" ? `${subject} (assumed)` : subject;

  if (face) {
    return (
      <img
        src={avatarUrl(subject)}
        alt={label}
        title={label}
        loading="lazy"
        className={`${SIZE_CLASS} object-cover`}
      />
    );
  }

  const hue = hueOf(subject);
  const named = readsAsName(subject);
  // `light-dark()` rather than a media query: `html` carries `color-scheme` under
  // both themes and under the forced override (`ui/global.css`), so one declaration
  // follows whichever the window is in.
  const disc = {
    backgroundColor: `light-dark(oklch(0.88 0.06 ${hue}), oklch(0.34 0.06 ${hue}))`,
    color: `light-dark(oklch(0.38 0.09 ${hue}), oklch(0.87 0.08 ${hue}))`,
  };
  return (
    <div
      className={`${SIZE_CLASS} flex items-center justify-center text-xs font-medium`}
      style={disc}
      title={label}
    >
      {named ? [...subject][0] : <Silhouette />}
    </div>
  );
}
