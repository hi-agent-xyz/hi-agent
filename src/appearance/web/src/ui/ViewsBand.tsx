import { useCallback, useEffect, useMemo, useState } from "react";
import { useViews } from "../core/views";
import { listViews, setBookmark, type ListedView } from "../channels/out/view";

/**
 * The views band — what has been shown, and where a person can go.
 *
 * **A band, not a panel.** It sits above the controls and is as short as its two rows
 * allow, because the most common reason to open it is to compare what is on the stage
 * with something that was there before, and a tall sheet would cover the very thing
 * being compared. Choosing dismisses it for the same reason.
 *
 * **Two rows, because there are two ways to want a view.** The upper row is history:
 * the raises the server recorded, oldest left, newest right, so going back is going
 * left and the live one is where the row ends. The lower row is bookmarks — the
 * surfaces we ship, plus whatever the person kept — which exists because a dozen
 * views shipped with no way to reach any of them except asking the agent to show it.
 *
 * **The lower row is not the inventory.** It was, and what is actually in the views
 * tree after a week of work is those shipped dozen plus every one-off a builder ever
 * wrote — `entry`, `entry b`, `entry mlat`, `mount b` — so the row read as a list of
 * the agent's scratch files with the surfaces a person wants buried among them. The
 * floor is now the system views alone, and anything else is there because the person
 * put it there: the star on a history card keeps it, the cross on a chip drops it.
 * Kept refs live in the config store, so they are the same on the desktop and the
 * phone — unlike the cursor, which is each window's own.
 *
 * **A flat strip rather than a cover flow.** The instinct behind a cover flow is right:
 * a view is remembered as a picture. But album art is square, uniform and *is* the
 * identity, while these are text-dense boards that at thumbnail size are all a grey
 * rectangle with rows — and a cover flow shows one item well and two in perspective, so
 * reading fifteen of them is a long drag. The strip keeps four to six legible at once
 * and swipes on a phone, where a perspective carousel is unusable.
 *
 * **Pictures, with the mark underneath.** The tile carries a real screenshot of the
 * raise, captured server-side the instant it went up by the same headless browser
 * `hi_review_view` drives — so it is a picture of the screen the person was looking
 * at, at the frame and in the skin their window reported. A capture that is still
 * running, or a view that did not render cleanly, leaves the tile on the coloured
 * mark derived from the view's identity, which is what the whole row used to be.
 */
export function ViewsBand({ onDismiss }: { onDismiss: () => void }) {
  const { history, parked, goTo, openRef } = useViews();
  const [inventory, setInventory] = useState<ListedView[]>([]);
  /** Shots whose `<img>` failed after the state said one existed — a shot pruned out
   * of the cache between the snapshot and the render. Falls back to the mark. */
  const [broken, setBroken] = useState<Set<string>>(() => new Set());

  useEffect(() => {
    let alive = true;
    void listViews().then(
      (found) => alive && setInventory(found),
      // An inventory that cannot be read leaves the row empty; history still works,
      // and the person is no worse off than before the band existed.
      (error) => console.warn("listing views failed", error),
    );
    return () => {
      alive = false;
    };
  }, []);

  /** What the inventory says about a ref — whether it is a system surface, and
   * whether it is kept. A ref that isn't here has no source on disk any more, so it
   * cannot be bookmarked and gets no star. */
  const known = useMemo(
    () => new Map(inventory.map((view) => [view.view_ref, view])),
    [inventory],
  );

  /** Applied to the row before the server answers: the row is the person's own state
   * and a star that waits on a round-trip reads as a dropped click. A failed write
   * puts it back, which is the only thing that could disagree with the store. */
  const keep = useCallback((viewRef: string, on: boolean) => {
    setInventory((current) =>
      current.map((v) => (v.view_ref === viewRef ? { ...v, bookmarked: on } : v)),
    );
    void setBookmark(viewRef, on).catch((error) => {
      console.warn("storing the bookmark failed", error);
      setInventory((current) =>
        current.map((v) => (v.view_ref === viewRef ? { ...v, bookmarked: !on } : v)),
      );
    });
  }, []);

  const liveKey = history.length > 0 ? destinationOf(history[history.length - 1]!) : null;
  const here = parked ?? liveKey;
  const bookmarks = inventory.filter((view) => view.system || view.bookmarked);

  return (
    <div className="hi-views-band" role="group" aria-label="views">
      <div className="hi-views-section">
        <span className="hi-views-heading">history</span>
        <span className="hi-views-rule" aria-hidden="true" />
      </div>
      {history.length === 0 ? (
        <p className="hi-views-empty">nothing has been shown yet</p>
      ) : (
        <div className="hi-views-strip">
          {history.map((entry) => {
            const key = destinationOf(entry);
            const isLive = key === liveKey;
            const shot = entry.shot_url && !broken.has(entry.shot_url) ? entry.shot_url : null;
            // Only a named view that is still on disk, and isn't already in the row by
            // being a system surface, is a thing the star can act on.
            const listed = entry.view_ref ? known.get(entry.view_ref) : undefined;
            const keepable = listed && !listed.system ? listed : null;
            return (
              <span className={`hi-views-card${key === here ? " is-here" : ""}`} key={key}>
                <button
                  type="button"
                  className="hi-views-open"
                  onClick={() => {
                    goTo(entry);
                    onDismiss();
                  }}
                  aria-current={key === here ? "true" : undefined}
                >
                  {/* The mark is painted whether or not there is a picture: it is the
                      ground a shot loads over, so a tile is never a hole in the row. */}
                  <span className="hi-views-tile" style={markStyle(key)} aria-hidden="true">
                    {shot ? (
                      <img
                        className="hi-views-shot"
                        src={shot}
                        alt=""
                        onError={() => setBroken((was) => new Set(was).add(shot))}
                      />
                    ) : (
                      initial(entry.label)
                    )}
                  </span>
                  <span className="hi-views-title">{entry.label}</span>
                  <span className="hi-views-when">
                    {isLive && <span className="hi-views-pip" aria-hidden="true" />}
                    {isLive ? "live" : shortTime(entry.at)}
                  </span>
                </button>
                {keepable && (
                  <button
                    type="button"
                    className={`hi-views-keep${keepable.bookmarked ? " is-kept" : ""}`}
                    aria-pressed={keepable.bookmarked}
                    aria-label={
                      keepable.bookmarked
                        ? `remove ${keepable.label} from bookmarks`
                        : `bookmark ${keepable.label}`
                    }
                    title={keepable.bookmarked ? "remove from bookmarks" : "bookmark"}
                    onClick={() => keep(keepable.view_ref, !keepable.bookmarked)}
                  >
                    <StarMark filled={keepable.bookmarked} />
                  </button>
                )}
              </span>
            );
          })}
        </div>
      )}

      <div className="hi-views-section">
        <span className="hi-views-heading">bookmarks</span>
        <span className="hi-views-rule" aria-hidden="true" />
      </div>
      <div className="hi-views-bookmarks">
        {bookmarks.map((view) => (
          <span
            className={`hi-views-chip${view.view_ref === here ? " is-here" : ""}`}
            key={view.view_ref}
          >
            <button
              type="button"
              className="hi-views-go"
              onClick={() => {
                openRef(view.view_ref);
                onDismiss();
              }}
            >
              <span className="hi-views-ico" style={markStyle(view.view_ref)} aria-hidden="true">
                {initial(view.label)}
              </span>
              {view.label}
            </button>
            {!view.system && (
              <button
                type="button"
                className="hi-views-drop"
                aria-label={`remove ${view.label} from bookmarks`}
                title="remove from bookmarks"
                onClick={() => keep(view.view_ref, false)}
              >
                <CrossMark />
              </button>
            )}
          </span>
        ))}
      </div>
    </div>
  );
}

/** The same destination identity the server dedupes by and the cursor is keyed on. */
function destinationOf(entry: { view_ref?: string; module_url: string }): string {
  return entry.view_ref ?? entry.module_url;
}

function StarMark({ filled }: { filled: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width="13" height="13" aria-hidden="true">
      <path
        d="M12 3.6l2.5 5.4 5.9.7-4.4 4 1.2 5.8L12 16.6 6.8 19.5 8 13.7 3.6 9.7l5.9-.7z"
        fill={filled ? "currentColor" : "none"}
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function CrossMark() {
  return (
    <svg viewBox="0 0 24 24" width="11" height="11" aria-hidden="true">
      <path
        d="M6 6l12 12M18 6L6 18"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.2"
        strokeLinecap="round"
      />
    </svg>
  );
}

function initial(label: string): string {
  return (label.trim()[0] ?? "?").toUpperCase();
}

/** A stable hue per destination, so a view keeps the same mark between sessions and
 * the row is scannable by colour before it is readable by label. Muted on purpose —
 * the band sits over the agent's screen and must not compete with it. */
function markStyle(key: string): { background: string } {
  let hash = 0;
  for (const ch of key) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  const hue = Math.abs(hash) % 360;
  return { background: `hsl(${hue} 34% 68%)` };
}

/** `14:05` today, `Mon` this week, `4 Aug` beyond — the resolution that tells the two
 * entries apart, which is all a timestamp is here for. */
function shortTime(iso: string): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return "";
  const now = new Date();
  const sameDay =
    at.getFullYear() === now.getFullYear() &&
    at.getMonth() === now.getMonth() &&
    at.getDate() === now.getDate();
  if (sameDay) {
    return at.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  }
  const days = (now.getTime() - at.getTime()) / 86_400_000;
  if (days < 7) return at.toLocaleDateString(undefined, { weekday: "short" });
  return at.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}
