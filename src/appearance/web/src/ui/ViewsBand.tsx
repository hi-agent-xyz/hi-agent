import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { destinationOf } from "../core/trail";
import { useViews } from "../core/views";
import { scrollToShow } from "../lib/strip";
import { listViews, setBookmark, type ListedView } from "../channels/out/view";

/**
 * The views band — what has been shown, and where a person can go.
 *
 * **A band, not a panel.** It sits above the controls and is as short as its two rows
 * allow, because the most common reason to open it is to compare what is on the stage
 * with something that was there before, and a tall sheet would cover the very thing
 * being compared. Choosing dismisses it for the same reason.
 *
 * **Two rows, because there are two ways to want a view.** The upper row is the trail:
 * where this window can go back to, **newest first** — the raises the server recorded
 * and the places the person opened themselves, one card per destination. Newest first
 * because the row overflows and only ever scrolls from its start, so oldest-first put
 * the live view, of all things, off the right-hand edge. The lower row is bookmarks —
 * the surfaces we ship, plus whatever the person kept — which exists because a dozen
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
 * **Pictures, with the mark underneath.** The tile carries a real screenshot, captured
 * server-side by the same headless browser `hi_review_view` drives, at the frame and in
 * the skin and language the window reported — a picture of the screen the person was
 * looking at, not a reconstruction. A named surface's picture is re-taken when they
 * open it, because the card leads to *today's* board and a tile promising last week's
 * would be a wrong picture of the place it goes. A capture that is still running, or a
 * view that did not render cleanly, leaves the tile on the coloured mark derived from
 * the view's identity, which is what the whole row used to be.
 *
 * **It opens on where you are.** The row can be a dozen cards long and the cursor is
 * not always at its head — a raise lands there, but a card gone back to keeps its place
 * — so opening the band scrolls the one marked *here* into view. Once, on opening: a
 * raise arriving afterwards must not drag the row out from under someone reading it,
 * which is the same refusal the return-to-live dot exists to make.
 *
 * **The inventory is re-read while the band is up.** A picture is only taken when
 * someone shows an interest in the view, and the first interest is usually the band
 * opening; the shot lands a second or two later, and this is what carries it onto the
 * card the person is already looking at.
 */
export function ViewsBand({ onDismiss }: { onDismiss: () => void }) {
  const { trail, live, parked, goTo, openRef } = useViews();
  const [inventory, setInventory] = useState<ListedView[]>([]);
  /** Shots whose `<img>` failed after the state said one existed — a shot pruned out
   * of the cache between the snapshot and the render. Falls back to the mark. */
  const [broken, setBroken] = useState<Set<string>>(() => new Set());

  const strip = useRef<HTMLDivElement>(null);
  const hereCard = useRef<HTMLElement>(null);
  /** Whether opening has already placed the row. The inventory poll re-renders the band
   * every few seconds, and re-running this would keep hauling the row back. */
  const placed = useRef(false);

  // Before paint, so the row is simply *at* the right place rather than seen to jump
  // there. `scrollLeft` rather than `scrollIntoView`, which would also scroll whatever
  // ancestor it decided was interesting, and would animate.
  useLayoutEffect(() => {
    if (placed.current || !strip.current || !hereCard.current) return;
    placed.current = true;
    const at = scrollToShow(strip.current, hereCard.current);
    if (at !== null) strip.current.scrollLeft = at;
  });

  /** Stars clicked whose write has not come back yet. A re-read that was already in
   * flight when the click happened answers with the old row, and applying it would
   * flick the star off under the finger and on again a poll later. */
  const inFlight = useRef(new Map<string, boolean>());

  useEffect(() => {
    let alive = true;
    const read = () =>
      void listViews().then(
        (found) =>
          alive &&
          setInventory(
            found.map((v) => {
              const pending = inFlight.current.get(v.view_ref);
              return pending === undefined ? v : { ...v, bookmarked: pending };
            }),
          ),
        // An inventory that cannot be read leaves the row empty; the trail still works,
        // and the person is no worse off than before the band existed.
        (error) => console.warn("listing views failed", error),
      );
    read();
    // Re-read while the band is up, for the pictures: a read is a directory walk and a
    // handful of `stat`s, and it stops the moment the band closes.
    const again = setInterval(read, INVENTORY_POLL_MS);
    return () => {
      alive = false;
      clearInterval(again);
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
    inFlight.current.set(viewRef, on);
    setInventory((current) =>
      current.map((v) => (v.view_ref === viewRef ? { ...v, bookmarked: on } : v)),
    );
    void setBookmark(viewRef, on)
      .catch((error) => {
        console.warn("storing the bookmark failed", error);
        setInventory((current) =>
          current.map((v) => (v.view_ref === viewRef ? { ...v, bookmarked: !on } : v)),
        );
      })
      .finally(() => inFlight.current.delete(viewRef));
  }, []);

  const here = parked ?? live;
  const bookmarks = inventory.filter((view) => view.system || view.bookmarked);

  return (
    <div className="hi-views-band" role="group" aria-label="views">
      <div className="hi-views-section">
        <span className="hi-views-heading">history</span>
        <span className="hi-views-rule" aria-hidden="true" />
      </div>
      {trail.length === 0 ? (
        <p className="hi-views-empty">nothing has been shown yet</p>
      ) : (
        <div className="hi-views-strip" ref={strip}>
          {trail.map((entry) => {
            const key = destinationOf(entry);
            const isLive = key === live;
            // The inventory wins when it has one: it is re-read while the band is up,
            // so it is the fresher of the two answers about a named surface's picture.
            const listed = entry.view_ref ? known.get(entry.view_ref) : undefined;
            const current = listed?.shot_url ?? entry.shot_url;
            const shot = current && !broken.has(current) ? current : null;
            // Only a named view that is still on disk, and isn't already in the row by
            // being a system surface, is a thing the star can act on.
            const keepable = listed && !listed.system ? listed : null;
            return (
              <span
                className={`hi-views-card${key === here ? " is-here" : ""}`}
                key={key}
                ref={key === here ? hereCard : undefined}
              >
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
                openRef(view.view_ref, view.label);
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

/** How often the band re-reads the inventory while it is up — long enough not to be a
 * poll anyone notices, short enough that a picture taken because the band opened lands
 * on the card before the person has finished reading the row. */
const INVENTORY_POLL_MS = 3000;

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
