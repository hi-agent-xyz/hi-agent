import { useEffect, useState } from "react";
import { useViews } from "../core/views";
import { listViews, type ListedView } from "../channels/out/view";

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
 * left and the live one is where the row ends. The lower row is the inventory — every
 * named view on disk — which exists because a dozen views shipped with no way to reach
 * any of them except asking the agent to show it.
 *
 * **A flat strip rather than a cover flow.** The instinct behind a cover flow is right:
 * a view is remembered as a picture. But album art is square, uniform and *is* the
 * identity, while these are text-dense boards that at thumbnail size are all a grey
 * rectangle with rows — and a cover flow shows one item well and two in perspective, so
 * reading fifteen of them is a long drag. The strip keeps four to six legible at once
 * and swipes on a phone, where a perspective carousel is unusable.
 *
 * **Marks, not screenshots.** A view is a live React app, not an image; capturing one
 * means either grabbing pixels at replace time (when the module may already be
 * unmounted) or re-mounting it offscreen (expensive, and for a named view it would
 * render *today*, contradicting the record it is standing in). A browser's own history
 * is a title, a favicon and a time — screenshots appear only in a mobile tab switcher,
 * where the set is small and current. So each tile carries a mark derived from the
 * view's identity, in the same box a thumbnail would later occupy.
 */
export function ViewsBand({ onDismiss }: { onDismiss: () => void }) {
  const { history, parked, goTo, openRef } = useViews();
  const [inventory, setInventory] = useState<ListedView[]>([]);

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

  const liveKey = history.length > 0 ? destinationOf(history[history.length - 1]!) : null;
  const here = parked ?? liveKey;

  return (
    <div className="hi-views-band" role="group" aria-label="views">
      <div className="hi-views-section">
        <span className="hi-views-heading">history</span>
        <span className="hi-views-rule" aria-hidden="true" />
        <span className="hi-views-hint" aria-hidden="true">
          newer →
        </span>
      </div>
      {history.length === 0 ? (
        <p className="hi-views-empty">nothing has been shown yet</p>
      ) : (
        <div className="hi-views-strip">
          {history.map((entry) => {
            const key = destinationOf(entry);
            const isLive = key === liveKey;
            return (
              <button
                type="button"
                key={key}
                className={`hi-views-card${key === here ? " is-here" : ""}`}
                onClick={() => {
                  goTo(entry);
                  onDismiss();
                }}
                aria-current={key === here ? "true" : undefined}
              >
                <span className="hi-views-tile" style={markStyle(key)} aria-hidden="true">
                  {initial(entry.label)}
                </span>
                <span className="hi-views-title">{entry.label}</span>
                <span className="hi-views-when">
                  {isLive && <span className="hi-views-pip" aria-hidden="true" />}
                  {isLive ? "live" : shortTime(entry.at)}
                </span>
              </button>
            );
          })}
        </div>
      )}

      <div className="hi-views-section">
        <span className="hi-views-heading">places</span>
        <span className="hi-views-rule" aria-hidden="true" />
      </div>
      <div className="hi-views-places">
        {inventory.map((view) => (
          <button
            type="button"
            key={view.view_ref}
            className={`hi-views-chip${view.view_ref === here ? " is-here" : ""}`}
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
        ))}
      </div>
    </div>
  );
}

/** The same destination identity the server dedupes by and the cursor is keyed on. */
function destinationOf(entry: { view_ref?: string; module_url: string }): string {
  return entry.view_ref ?? entry.module_url;
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
