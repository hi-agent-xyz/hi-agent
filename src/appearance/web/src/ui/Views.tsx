import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { destinationOf } from "../core/trail";
import { useViews } from "../core/views";
import { listViews, setBookmark, type ListedView } from "../channels/out/view";

/**
 * The views tab — what has been shown, and where a person can go.
 *
 * **A tab, not a band.** It was a short strip floating above the channel controls
 * on a window and a pushed page on a phone, and the shortness was argued for: the
 * most common reason to open it is to compare what is on the stage with something
 * that was there before, and a tall sheet would cover the very thing being
 * compared. That argument is answered by the axis rather than by the height now
 * (`docs/arch/stage.md` § *The panel, and the axis it runs on*) — at the middle
 * stop the panel sits *beside* what it is being compared against rather than over
 * it, so the tab can have the panel's whole body and still not cover anything.
 * The band-versus-page split goes with it: one tab, laid out by how much room the
 * stop gives it.
 *
 * **Two rows, because there are two ways to want a view.** The upper row is the trail:
 * where the screen can go back to, **newest first** — the shows the server recorded and
 * the places the person opened themselves, in one list, one card per destination.
 * Tapping a card moves the screen for every attached window, not this one. Newest first
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
 * phone — unlike the stop, which is each window's own.
 *
 * **A flat grid rather than a cover flow.** The instinct behind a cover flow is right:
 * a view is remembered as a picture. But album art is square, uniform and *is* the
 * identity, while these are text-dense boards that at thumbnail size are all a grey
 * rectangle with rows — and a cover flow shows one item well and two in perspective, so
 * reading fifteen of them is a long drag. A grid of pictures is legible all at once and
 * needs no gesture at all to read, on a phone as much as on a window.
 *
 * **Pictures, with the name written on them.** The tile carries a real screenshot,
 * captured server-side by the same headless browser `hi_review_view` drives, at the
 * frame and in the skin and language the window reported — a picture of the screen the
 * person was looking at, not a reconstruction. A named surface's picture is re-taken
 * when they open it, because the card leads to *today's* board and a tile promising
 * last week's would be a wrong picture of the place it goes. A capture that is still
 * running, or a view that did not render cleanly, leaves the tile on the coloured mark
 * derived from the view's identity, which is what the whole row used to be.
 *
 * The card *is* the picture: the label sits on the shot's bottom edge over a gradient
 * rather than in a line below it, so every pixel of the card's height is the thing that
 * identifies the view, and nothing is drawn around it — where the screen is now is
 * marked by tinting that caption, not by a ring, because a frame around a screenshot
 * reads as part of the screenshot. The show time is not printed at all — the row is
 * already ordered by it, and nobody asks for a view by when it was put up.
 *
 * **It opens on where you are.** Both rows can be a dozen items long and the cursor is
 * not always at the head of either — a show lands there, but a card gone back to keeps
 * its place, and a bookmark can sit anywhere in the lower row — so opening the tab
 * scrolls whichever item is marked *here* into view, in the row that holds it. Once, on
 * opening: a show arriving afterwards must not drag a row out from under someone
 * reading it. The stage does follow a show; the row someone is reading does not.
 * Each row is its own scroller, so *in the row that holds it* is now literal — placing
 * the trail cannot move the bookmarks, and neither one can move the other's heading.
 *
 * **The inventory is re-read while the tab is up.** A picture is only taken when
 * someone shows an interest in the view, and the first interest is usually this tab
 * being opened; the shot lands a second or two later, and this is what carries it onto
 * the card the person is already looking at.
 *
 * **One layout, and the stop only says how big a picture gets.** Both rows were strips
 * that scrolled sideways at the panel's measure and a wrapping grid with the panel
 * across the whole window, and the sidebar was the loser of that split: four cards on
 * screen, the rest off the right-hand edge, and two thirds of the tab's body empty
 * under them. The room the rows were short of was never across — the tab has the
 * panel's whole body at every stop. So the grid runs at both measures and the
 * stylesheet changes nothing but the track (`ui/global.css`). Nothing here branches on
 * the stop any more, which is why there is no prop for it.
 *
 * **Both sections are present before anything is scrolled.** Wrapping the rows made the
 * trail as tall as the trail is, which put the bookmarks — the ten places a person
 * actually goes — a screen and a half down. So the tab is a column rather than one long
 * page: the bookmarks take the height their chips need at the foot, and the trail
 * scrolls in what is left above them (`ui/global.css`). Nothing in this file arranges
 * that; it is worth knowing here only because it is why each row scrolls on its own.
 */
export function Views({ onChose }: { onChose: () => void }) {
  const { trail, live, parked, goTo, openRef } = useViews();
  const [inventory, setInventory] = useState<ListedView[]>([]);
  /** Shots whose `<img>` failed after the state said one existed — a shot pruned out
   * of the cache between the snapshot and the render. Falls back to the mark. */
  const [broken, setBroken] = useState<Set<string>>(() => new Set());

  const hereCard = useRef<HTMLElement>(null);
  const hereChip = useRef<HTMLElement>(null);
  /** Whether opening has already placed each row. The inventory poll re-renders the tab
   * every few seconds, and re-running this would keep hauling a row back. One flag per
   * row because the two are filled from different sources: the trail is in context on the
   * first render, the bookmarks arrive with the first `listViews()`. */
  const placedCards = useRef(false);
  const placedChips = useRef(false);

  // Before paint, so a row is simply *at* the right place rather than seen to jump
  // there. `scrollIntoView` is the right tool now that each row is its own scroller
  // and nothing scrolls across: the ancestor it would otherwise move by surprise *is*
  // the row that has to move, and the tab around it cannot scroll at all.
  useLayoutEffect(() => {
    show(hereCard.current, placedCards);
    show(hereChip.current, placedChips);
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
        // and the person is no worse off than before the tab existed.
        (error) => console.warn("listing views failed", error),
      );
    read();
    // Re-read while the tab is up, for the pictures: a read is a directory walk and a
    // handful of `stat`s, and it stops the moment the tab is left.
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
    <div className="hi-views" role="group" aria-label="views">
      <div className="hi-views-section">
        <span className="hi-views-heading">history</span>
        <span className="hi-views-rule" aria-hidden="true" />
      </div>
      {trail.length === 0 ? (
        <p className="hi-views-empty">nothing has been shown yet</p>
      ) : (
        <div className="hi-views-strip">
          {trail.map((entry) => {
            const key = destinationOf(entry);
            const isLive = key === live;
                    // The inventory wins when it has one: it is re-read while the tab is up,
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
                    onChose();
                  }}
                  aria-current={key === here ? "true" : undefined}
                >
                  {/* The mark is painted whether or not there is a picture: it is the
                      ground a shot loads over, so a tile is never a hole in the row. The
                      label rides on the tile, so the tile is not `aria-hidden` — it
                      carries the button's whole accessible name. */}
                  <span className="hi-views-tile" style={markStyle(key)}>
                    {shot ? (
                      <img
                        className="hi-views-shot"
                        // Used as it arrives: the core resolves a shot against the
                        // base path the request came in on before handing it over
                        // (`foundation::surfaces::reroot_path`). It has to be done
                        // there and not here — an `<img src>` is not carried by the
                        // `fetch` seam that rebases everything else, and every reader
                        // that was asked to remember eventually forgot, this one
                        // included: the whole row fell back to its mark on a phone.
                        src={shot}
                        alt=""
                        onError={() => setBroken((was) => new Set(was).add(shot))}
                      />
                    ) : (
                      <span className="hi-views-mark" aria-hidden="true">
                        {initial(entry.label)}
                      </span>
                    )}
                    <span className="hi-views-title">
                      {isLive && <span className="hi-views-pip" aria-hidden="true" />}
                      <span className="hi-views-name">{entry.label}</span>
                    </span>
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
            ref={view.view_ref === here ? hereChip : undefined}
          >
            <button
              type="button"
              className="hi-views-go"
              onClick={() => {
                openRef(view.view_ref);
                onChose();
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

/** Bring the item marked *here* onto the screen, once, by scrolling the tab's body
 * down to it. `center` rather than `start`, because an item at the very top of the
 * frame reads as the head of the list and this one is somewhere in the middle of one. */
function show(item: HTMLElement | null, done: { current: boolean }): void {
  if (done.current || !item) return;
  done.current = true;
  item.scrollIntoView({ block: "center" });
}

/** How often the tab re-reads the inventory while it is up — long enough not to be a
 * poll anyone notices, short enough that a picture taken because the tab opened lands
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
 * the tab sits beside the agent's screen and must not compete with it. */
function markStyle(key: string): { background: string } {
  let hash = 0;
  for (const ch of key) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  const hue = Math.abs(hash) % 360;
  return { background: `hsl(${hue} 34% 68%)` };
}
