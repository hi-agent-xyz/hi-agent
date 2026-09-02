import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { destinationOf } from "../core/trail";
import { useViews } from "../core/views";
import { url } from "../lib/base";
import { scrollToShow } from "../lib/strip";
import { useIsPhone } from "../lib/shape";
import { BackGlyph } from "./ChannelControls";
import { PageEdge } from "./PageEdge";
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
 * phone — unlike the cursor, which is each window's own.
 *
 * **A flat strip rather than a cover flow.** The instinct behind a cover flow is right:
 * a view is remembered as a picture. But album art is square, uniform and *is* the
 * identity, while these are text-dense boards that at thumbnail size are all a grey
 * rectangle with rows — and a cover flow shows one item well and two in perspective, so
 * reading fifteen of them is a long drag. The strip keeps four or five legible at once
 * and swipes on a phone, where a perspective carousel is unusable.
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
 * its place, and a bookmark can sit anywhere in the lower row — so opening the band
 * scrolls whichever item is marked *here* into view, in the row that holds it. Once, on
 * opening: a show arriving afterwards must not drag a row out from under someone
 * reading it. The stage does follow a show; the row someone is reading does not.
 *
 * **The inventory is re-read while the band is up.** A picture is only taken when
 * someone shows an interest in the view, and the first interest is usually the band
 * opening; the shot lands a second or two later, and this is what carries it onto the
 * card the person is already looking at.
 *
 * **On the phone it is not a band.** Everything above is an argument about a
 * surface floating over a window that has room for both it and the thing being
 * compared, and a 390px screen has no such room: a band there is a short letterbox
 * of a strip with two cards in it, sitting on top of the view it was opened to
 * compare against and covering most of it anyway. So on the phone shape it is a
 * page pushed onto the stack, swiped back off the same way the conversation is
 * (`docs/arch/stage.md`), and the two rows spend the whole screen — history as a
 * grid of pictures big enough to recognise, bookmarks as chips that wrap. The
 * shortness was the compromise, not the goal; the goal was *what has been shown,
 * and where you can go*, which a page serves better than a strip.
 */
export function ViewsBand({ onDismiss }: { onDismiss: () => void }) {
  const { trail, live, parked, goTo, openRef } = useViews();
  const phone = useIsPhone();
  const [inventory, setInventory] = useState<ListedView[]>([]);
  /** Shots whose `<img>` failed after the state said one existed — a shot pruned out
   * of the cache between the snapshot and the render. Falls back to the mark. */
  const [broken, setBroken] = useState<Set<string>>(() => new Set());

  const strip = useRef<HTMLDivElement>(null);
  const hereCard = useRef<HTMLElement>(null);
  const chips = useRef<HTMLDivElement>(null);
  const hereChip = useRef<HTMLElement>(null);
  /** Whether opening has already placed each row. The inventory poll re-renders the band
   * every few seconds, and re-running this would keep hauling a row back. One flag per
   * row because the two are filled from different sources: the trail is in context on the
   * first render, the bookmarks arrive with the first `listViews()`. */
  const placedCards = useRef(false);
  const placedChips = useRef(false);

  // Before paint, so a row is simply *at* the right place rather than seen to jump
  // there. `scrollLeft` rather than `scrollIntoView`, which would also scroll whatever
  // ancestor it decided was interesting, and would animate.
  useLayoutEffect(() => {
    if (phone) {
      // The page scrolls down, not the rows across, so the same job is a vertical
      // one and `scrollIntoView` is the right tool for it here: the ancestor it
      // would otherwise scroll by surprise *is* the page, which is the box that
      // has to move. Still once, and still for the same reason.
      show(hereCard.current, placedCards);
      show(hereChip.current, placedChips);
      return;
    }
    place(strip.current, hereCard.current, placedCards);
    place(chips.current, hereChip.current, placedChips);
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
    <div
      className="hi-views-band"
      data-page={phone ? "true" : undefined}
      role="group"
      aria-label="views"
    >
      {phone && (
        <>
          {/* The page's bar. One control, and it is the way back — there is
              nothing else a person does to this page except leave it or pick
              something off it. */}
          <div className="hi-views-bar">
            <button
              type="button"
              className="hi-channel hi-channel--back"
              onClick={onDismiss}
              aria-label="back"
            >
              <BackGlyph />
            </button>
            <span className="hi-views-bar-title">Views</span>
          </div>
          <PageEdge onBack={onDismiss} />
        </>
      )}
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
                      ground a shot loads over, so a tile is never a hole in the row. The
                      label rides on the tile, so the tile is not `aria-hidden` — it
                      carries the button's whole accessible name. */}
                  <span className="hi-views-tile" style={markStyle(key)}>
                    {shot ? (
                      <img
                        className="hi-views-shot"
                        // The backend hands back a root-absolute `/views/_shots/…`,
                        // and an `<img src>` is not carried by the `fetch` seam that
                        // rebases everything else — so under the community's subpath
                        // this asked the community for the picture and every tile in
                        // the row fell back to its mark.
                        src={url(shot)}
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
      <div className="hi-views-bookmarks" ref={chips}>
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

/** Scroll one row so the item marked *here* is on screen, once. A row the browser has
 * not measured yet (`clientWidth` 0 — the band is mounted, layout has not reached it)
 * is left for a later render: `scrollToShow` on a zero-width box answers with a number,
 * and it is the wrong one. */
function place(
  row: HTMLDivElement | null,
  item: HTMLElement | null,
  done: { current: boolean },
): void {
  if (done.current || !row || !item || row.clientWidth === 0) return;
  done.current = true;
  const at = scrollToShow(row, item);
  if (at !== null) row.scrollLeft = at;
}

/** The page's version of the same job: bring the item marked *here* onto the
 * screen, once, by scrolling the page down to it. `center` rather than `start`,
 * because a card at the very top of the frame reads as the head of the list and
 * this one is somewhere in the middle of one. */
function show(item: HTMLElement | null, done: { current: boolean }): void {
  if (done.current || !item) return;
  done.current = true;
  item.scrollIntoView({ block: "center" });
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
