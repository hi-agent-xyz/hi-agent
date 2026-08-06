import { useEffect, useState } from "react";

interface EnergyStatus {
  out_of_energy: boolean;
  resets_in?: string;
}

// Where 升级 points before the signed-in link is minted (and if minting fails):
// the plain account page, which just asks the user to sign in.
const FALLBACK_URL = "https://hi.xiaoyuanzhu.com/account";

/**
 * The out-of-energy hint — a small host-chrome card pinned just above the channel
 * controls (same width, so it sits centered over them). It polls
 * `/api/account/energy`; while the account is out of energy it shows a quiet
 * reassurance — you can keep typing, nothing is lost, processing just waits — and an
 * 升级 link to the account page **already signed in as this device account**. It hides
 * itself the moment energy refills. No spoken nudge: this quiet corner card replaces
 * the old 402 message.
 *
 * 升级 is a real `<a target="_blank">`, not a fetch-then-`window.open`: the signed-in
 * URL is minted once when the hint appears, so the click opens it directly within the
 * user gesture. That matters in the desktop face window (a `WKWebView`) — an async
 * `window.open` after `await` loses the user-activation and WebKit blocks it (nothing
 * opens); the native `WKUIDelegate` then routes the new-window request to the system
 * browser.
 */
export function OutOfEnergyHint() {
  const [out, setOut] = useState(false);
  const [resetsIn, setResetsIn] = useState("");
  const [href, setHref] = useState(FALLBACK_URL);

  // Poll sequentially (fetch → wait 5s), so a slow request never overlaps the next
  // tick. While paused the endpoint refreshes the real broker balance; a positive
  // result emits Resume in the host. This remains alive with the app window hidden,
  // because recovery belongs to the running app, not to a foreground gesture.
  useEffect(() => {
    let alive = true;
    let knownOut = false;
    let timer: number | undefined;
    const schedule = () => {
      if (alive) timer = window.setTimeout(run, 5000);
    };
    const run = async () => {
      timer = undefined;
      try {
        // Normal reads are cheap. Once a 402 has raised the card, each poll
        // refreshes the real broker balance; the first positive balance broadcasts
        // Resume inside the host and wakes every held agent session.
        const url = knownOut
          ? "/api/account/energy?refresh=true"
          : "/api/account/energy";
        const r = await fetch(url);
        if (r.ok) {
          const d: EnergyStatus = await r.json();
          if (alive) {
            knownOut = !!d.out_of_energy;
            setOut(knownOut);
            setResetsIn(d.resets_in ?? "");
          }
        }
      } catch {
        /* transient — try again on the next tick */
      }
      schedule();
    };
    run();
    return () => {
      alive = false;
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, []);

  // Mint a fresh signed-in link once, when the hint appears. Keeping it pre-resolved
  // lets 升级 be a plain anchor that opens in the same click (see the note above).
  useEffect(() => {
    if (!out) return;
    let alive = true;
    fetch("/api/account/subscribe")
      .then((r) => r.json())
      .then((d) => {
        if (alive && d?.url) setHref(d.url);
      })
      .catch(() => {
        /* keep the fallback URL */
      });
    return () => {
      alive = false;
    };
  }, [out]);

  if (!out) return null;

  return (
    <div className="hi-oe" role="status" aria-live="polite">
      <div className="hi-oe-head">
        <span className="hi-oe-spark" aria-hidden>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none">
            <path
              d="M13 2 4 14h6l-1 8 9-12h-6l1-8Z"
              fill="#fd605e"
              stroke="rgba(0,0,0,0.06)"
              strokeWidth="0.5"
            />
          </svg>
        </span>
        <span className="hi-oe-title">能量用完了</span>
      </div>
      <div className="hi-oe-body">
        可以<b>继续输入，消息不会丢</b>——等能量恢复我就接着处理。
      </div>
      <div className="hi-oe-foot">
        <span className="hi-oe-reset">
          {resetsIn ? `${resetsIn}恢复` : "很快恢复"}
        </span>
        <a
          className="hi-oe-btn"
          href={href}
          target="_blank"
          rel="noopener noreferrer"
        >
          升级
        </a>
      </div>
    </div>
  );
}
