import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import {
  shareState,
  shareView,
  ShareRefused,
  unshareView,
  type Published,
  type ShareKind,
  type ShareState,
} from "../channels/out/view";
import { placeMenu } from "../lib/longPress";

/** What a card's menu is about. */
export interface MenuTarget {
  ref: string;
  label: string;
  /** The tile's picture, for the share sheet to show what is about to go out. */
  shot: string | null;
  /** Where it was opened — the pointer, or the card's corner from the keyboard. */
  x: number;
  y: number;
  /** A view in the tree can be kept and deleted; an attachment on the trail can only be
   * shared. A system view gets no menu at all — see `Views.tsx`. */
  kind: "view" | "attachment";
  bookmarked: boolean;
  shared?: ShareKind;
}

/**
 * The card's menu: share, bookmark, delete.
 *
 * **Drawn on the cover plane, not inside the panel.** The panel is a transformed box that
 * clips its overflow, so a menu inside it would be positioned against the panel rather than
 * the pointer and cut off at its edge. Portalled onto the plane the panel sits on, it is
 * still the person's chrome — the key routing reads it as `cover` (`lib/keyboard.ts`) — and
 * it is over everything else there by being last.
 *
 * **Escape is answered here and consumed**, by `preventDefault` in the menu's own keydown:
 * the shell's Escape puts the panel away, and it stands down for a key a surface already
 * acted on (`onHostKey`'s contract). React's handlers run before the document's, so this is
 * the first to hear it, and the shell never sees a live one.
 *
 * Delete asks first, in place, because it is the one item that takes something away.
 */
export function ViewMenu({
  target,
  onClose,
  onShare,
  onBookmark,
  onDelete,
}: {
  target: MenuTarget;
  onClose: () => void;
  onShare: () => void;
  onBookmark: (on: boolean) => void;
  onDelete: () => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [at, setAt] = useState<{ left: number; top: number } | null>(null);
  const [confirming, setConfirming] = useState(false);

  // A held press opens the menu with its corner under the finger, and the finger lifting
  // can land a click on whichever item is there. A click that soon after opening is that
  // finger, not a choice; the keyboard's clicks (`detail` 0) are always choices.
  const opened = useRef(performance.now());
  const chosen = (act: () => void) => (event: { detail: number }) => {
    if (event.detail > 0 && performance.now() - opened.current < LIFT_MS) return;
    act();
  };

  // Measured before paint and placed against the screen, so a menu opened by the right
  // edge opens leftwards rather than being seen to jump there.
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    setAt(placeMenu(target.x, target.y, width, height, window.innerWidth, window.innerHeight));
  }, [target.x, target.y, confirming]);

  // The first item has the focus, so the keyboard is in the menu the moment it is up.
  // Once it is placed: until then it is `visibility: hidden`, which cannot hold a focus.
  const placed = at !== null;
  useEffect(() => {
    if (placed) box.current?.querySelector<HTMLElement>("[role=menuitem]")?.focus();
  }, [confirming, placed]);

  useDismiss(box, onClose);

  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      if (confirming) setConfirming(false);
      else onClose();
      return;
    }
    if (event.key === "Tab") {
      onClose();
      return;
    }
    const items = [...(box.current?.querySelectorAll<HTMLElement>("[role=menuitem]") ?? [])];
    const now = items.indexOf(document.activeElement as HTMLElement);
    const next =
      event.key === "ArrowDown" ? (now + 1) % items.length
      : event.key === "ArrowUp" ? (now - 1 + items.length) % items.length
      : event.key === "Home" ? 0
      : event.key === "End" ? items.length - 1
      : null;
    if (next === null) return;
    event.preventDefault();
    items[next]?.focus();
  };

  const isView = target.kind === "view";
  const shareLabel =
    target.shared === "public" ? "shared · public"
    : target.shared === "unlisted" ? "shared · unlisted"
    : "share";

  return onCover(
    <div
      ref={box}
      className="hi-menu"
      role="menu"
      aria-label={target.label}
      // Focusable itself, so a click on its padding keeps the focus inside it — and with
      // it the keys, Escape above all, which would otherwise go to the shell.
      tabIndex={-1}
      style={at ? { left: at.left, top: at.top } : { left: target.x, top: target.y, visibility: "hidden" }}
      onKeyDown={keys}
      onContextMenu={(e) => e.preventDefault()}
    >
      {confirming ? (
        <>
          <p className="hi-menu-note">
            delete <b>{target.label}</b>? it goes to the trash, and its cards leave history.
            {target.shared && " its link stops working."}
          </p>
          <button type="button" role="menuitem" className="hi-menu-item is-danger" onClick={chosen(onDelete)}>
            delete
          </button>
          <button type="button" role="menuitem" className="hi-menu-item" onClick={chosen(() => setConfirming(false))}>
            cancel
          </button>
        </>
      ) : (
        <>
          <button type="button" role="menuitem" className="hi-menu-item" onClick={chosen(onShare)}>
            {shareLabel}
          </button>
          {isView && (
            <button
              type="button"
              role="menuitem"
              className="hi-menu-item"
              onClick={chosen(() => onBookmark(!target.bookmarked))}
            >
              {target.bookmarked ? "remove from bookmarks" : "bookmark"}
            </button>
          )}
          {isView && (
            <button
              type="button"
              role="menuitem"
              className="hi-menu-item is-danger"
              onClick={chosen(() => setConfirming(true))}
            >
              delete…
            </button>
          )}
        </>
      )}
    </div>,
  );
}

/** How long after opening a pointer's click is still the press that opened it. */
const LIFT_MS = 350;

type Sheet =
  | { at: "reading" }
  | { at: "choosing"; unlisted: boolean }
  | { at: "checking"; unlisted: boolean }
  | { at: "published"; published: Published; unlisted: boolean }
  | { at: "standing"; state: ShareState }
  | { at: "refused"; refusals: string[]; unlisted: boolean }
  | { at: "stopped" };

/**
 * Sharing one view or attachment, from its card.
 *
 * **The owner sees what is about to go out** — the tile's picture heads the sheet
 * (`docs/arch/sharing.md` § *The check*): a check that passes does not mean the content
 * was meant to leave. And the sheet says, before anything is made, that **whoever holds
 * the link can open it** — an unlisted link is not private, only unguessable, and the
 * design insists the UI says so rather than implying a permission.
 *
 * Unlisted is the default. A shared view is re-read from the server when the sheet opens,
 * because that is the only thing that knows its link: a public link can be copied again,
 * an unlisted one cannot be rebuilt from the key's hash and can only be replaced — which
 * retires the old key, so the sheet says the old link stops working.
 *
 * Every address shown here is the server's. A core with no name hands back a path on this
 * machine and says so, and the sheet passes that on in words rather than building a URL
 * from whatever host this window happens to be on.
 */
export function ShareSheet({
  target,
  onClose,
  onChanged,
}: {
  target: Pick<MenuTarget, "ref" | "label" | "shot">;
  onClose: () => void;
  /** The share state moved, so the inventory's word for it is stale. */
  onChanged: (shared: ShareKind | undefined) => void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [sheet, setSheet] = useState<Sheet>({ at: "reading" });
  /** The last request failed for a reason that is not the owner's to fix. Said in one
   *  line; what went wrong is in the console, not on the sheet. */
  const [trouble, setTrouble] = useState(false);

  useEffect(() => {
    let alive = true;
    shareState(target.ref).then(
      (state) => alive && setSheet(state ? { at: "standing", state } : { at: "choosing", unlisted: true }),
      (error) => {
        console.warn("reading the share failed", error);
        if (alive) setSheet({ at: "choosing", unlisted: true });
      },
    );
    return () => {
      alive = false;
    };
  }, [target.ref]);

  useEffect(() => {
    const el = box.current;
    (el?.querySelector<HTMLElement>(".is-primary, .hi-sheet-link button, .hi-sheet-actions button") ?? el)?.focus();
  }, [sheet.at]);

  const publish = (unlisted: boolean) => {
    setTrouble(false);
    setSheet({ at: "checking", unlisted });
    shareView(target.ref, { unlisted }).then(
      (published) => {
        setSheet({ at: "published", published, unlisted });
        onChanged(unlisted ? "unlisted" : "public");
      },
      (error: unknown) => {
        if (error instanceof ShareRefused) {
          setSheet({ at: "refused", refusals: error.refusals, unlisted });
          return;
        }
        console.warn("sharing failed", error);
        setTrouble(true);
        setSheet({ at: "choosing", unlisted });
      },
    );
  };

  const stop = () => {
    setTrouble(false);
    unshareView(target.ref).then(
      () => {
        setSheet({ at: "stopped" });
        onChanged(undefined);
      },
      (error) => {
        console.warn("stopping the share failed", error);
        setTrouble(true);
      },
    );
  };

  const keys = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    }
  };

  return onCover(
    <div className="hi-sheet-scrim" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        ref={box}
        className="hi-sheet"
        role="dialog"
        aria-modal="true"
        aria-label={`share ${target.label}`}
        tabIndex={-1}
        onKeyDown={keys}
      >
        <div className="hi-sheet-head">
          <span className="hi-sheet-title">share {target.label}</span>
          <button type="button" className="hi-sheet-close" aria-label="close" onClick={onClose}>
            ×
          </button>
        </div>
        {target.shot && <img className="hi-sheet-shot" src={target.shot} alt="" />}

        {sheet.at === "reading" && <p className="hi-sheet-note">…</p>}

        {(sheet.at === "choosing" || sheet.at === "refused") && (
          <>
            <p className="hi-sheet-note">anyone with the link can open it — a link is not a password.</p>
            <fieldset className="hi-sheet-kinds">
              <legend className="hi-sheet-legend">who can find it</legend>
              <Kind
                checked={sheet.unlisted}
                onPick={() => setSheet({ at: "choosing", unlisted: true })}
                title="unlisted"
                detail="only people you give the link to"
              />
              <Kind
                checked={!sheet.unlisted}
                onPick={() => setSheet({ at: "choosing", unlisted: false })}
                title="public"
                detail="anyone who has the address, search engines included"
              />
            </fieldset>
            {sheet.at === "refused" && (
              <div className="hi-sheet-refused" role="alert">
                <p>it can't be shared as it is:</p>
                <ul>
                  {sheet.refusals.map((why) => (
                    <li key={why}>{why}</li>
                  ))}
                </ul>
              </div>
            )}
            <div className="hi-sheet-actions">
              <button type="button" className="hi-sheet-button is-primary" onClick={() => publish(sheet.unlisted)}>
                create link
              </button>
            </div>
          </>
        )}

        {sheet.at === "checking" && (
          <p className="hi-sheet-note hi-sheet-busy" role="status">
            checking how it looks to someone without access…
          </p>
        )}

        {sheet.at === "published" && (
          <>
            <Link link={sheet.published.link} />
            <Reachable reach={sheet.published.reachable} />
            <p className="hi-sheet-note">
              {sheet.unlisted
                ? "the key is in this link and is not shown again. anyone with the link can open it."
                : "anyone with the address can open it."}
            </p>
            <div className="hi-sheet-actions">
              <button type="button" className="hi-sheet-button" onClick={onClose}>
                done
              </button>
            </div>
          </>
        )}

        {sheet.at === "standing" && (
          <>
            {sheet.state.kind === "public" && sheet.state.link ? (
              <>
                <p className="hi-sheet-note">shared publicly — anyone with the address can open it.</p>
                <Link link={sheet.state.link} />
              </>
            ) : (
              <p className="hi-sheet-note">
                shared as an unlisted link. its key isn't kept, so the link can't be shown again — a
                new link replaces it, and the old one stops working.
              </p>
            )}
            <Reachable reach={sheet.state.reachable} />
            <div className="hi-sheet-actions">
              {sheet.state.kind === "unlisted" && (
                <button type="button" className="hi-sheet-button is-primary" onClick={() => publish(true)}>
                  new link
                </button>
              )}
              <button type="button" className="hi-sheet-button is-danger" onClick={stop}>
                stop sharing
              </button>
            </div>
            <p className="hi-sheet-fine">a stopped link can keep working for about a minute.</p>
          </>
        )}

        {trouble && (
          <p className="hi-sheet-refused" role="alert">
            that didn't go through. try again in a moment.
          </p>
        )}

        {sheet.at === "stopped" && (
          <>
            <p className="hi-sheet-note">
              no longer shared. a copy already out there can keep working for about a minute.
            </p>
            <div className="hi-sheet-actions">
              <button type="button" className="hi-sheet-button" onClick={onClose}>
                done
              </button>
            </div>
          </>
        )}
      </div>
    </div>,
  );
}

function Kind({
  checked,
  onPick,
  title,
  detail,
}: {
  checked: boolean;
  onPick: () => void;
  title: string;
  detail: string;
}) {
  return (
    <label className={`hi-sheet-kind${checked ? " is-on" : ""}`}>
      <input type="radio" name="share-kind" checked={checked} onChange={onPick} />
      <span>
        <span className="hi-sheet-kind-title">{title}</span>
        <span className="hi-sheet-kind-detail">{detail}</span>
      </span>
    </label>
  );
}

/** The link, selectable, with a copy button. */
function Link({ link }: { link: string }) {
  const field = useRef<HTMLInputElement>(null);
  const [copied, setCopied] = useState(false);
  const copy = () => {
    // The clipboard API needs a secure context, which a phone reaching this core over
    // plain http on the LAN is not; selecting the text leaves the system's own copy.
    const fallback = () => field.current?.select();
    if (navigator.clipboard?.writeText) {
      navigator.clipboard.writeText(link).then(() => setCopied(true), fallback);
    } else {
      fallback();
    }
  };
  return (
    <div className="hi-sheet-link">
      <input ref={field} className="hi-sheet-url" readOnly value={link} onFocus={(e) => e.target.select()} />
      <button type="button" className="hi-sheet-button" onClick={copy}>
        {copied ? "copied" : "copy"}
      </button>
    </div>
  );
}

function Reachable({ reach }: { reach: ShareState["reachable"] }) {
  if (reach === "anywhere") return null;
  return (
    <p className="hi-sheet-note is-warn">
      this agent has no name yet, so the link only works on this machine. claim a handle in settings to
      make it reachable from anywhere.
    </p>
  );
}

/** Close on a pointer going down anywhere outside `box`, and on any scroll outside it —
 *  a menu left floating over a row that has moved points at the wrong card. */
function useDismiss(box: RefObject<HTMLElement | null>, onClose: () => void) {
  useEffect(() => {
    const outside = (event: Event) => {
      if (box.current && event.target instanceof Node && box.current.contains(event.target)) return;
      onClose();
    };
    document.addEventListener("pointerdown", outside, { capture: true });
    document.addEventListener("scroll", outside, { capture: true });
    window.addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("pointerdown", outside, { capture: true });
      document.removeEventListener("scroll", outside, { capture: true });
      window.removeEventListener("resize", onClose);
    };
  }, [box, onClose]);
}

/** Into the person's plane, beside the panel rather than inside it. Falls back to the body
 *  where there is no plane (a test, or a page with no shell). */
function onCover(node: ReactNode) {
  return createPortal(node, document.querySelector(".hi-plane--cover") ?? document.body);
}
