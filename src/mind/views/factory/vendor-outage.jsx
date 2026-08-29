// purpose: the host's own notice that managed calls are paused on a 402. Shown and taken down by the vendor gate, not by the agent.
// Host-owned condition state, not model output. The process-wide vendor gate shows and hides it directly
// from the durable energy level and polls recovery while it is visible.
//
// "You are out" is the least useful half of what a person wants at this moment. The
// other half is the account it happened on, the plan that set the ceiling, where the
// day's energy went, and how long the wait is — so this view answers all of them:
// account + tier + remaining/total (with a Refresh next to it), the last 24h of
// observed level drawn like a battery graph, a live countdown to the window reset,
// and only then the way out. It carries no explanatory prose: every line here is a
// number or a name, and reassurance the person didn't ask for is what made the
// earlier version feel long.
//
// The whole screen is the panel — one surface, not a card floating under a heading.
// The chart takes whatever height is left over, so the view fills a desktop window
// without a scrollbar and still holds together on a phone.
//
// It reads `GET /api/account/energy?history=true` — the same endpoint the gate polls,
// plus the sampled series ([crate::foundation::energy_history]). Gaps in that series
// are hours the app wasn't running; they are drawn as gaps, never bridged.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { motion } from "motion/react";

const FALLBACK_URL = "https://hi-agent.xyz/account";
const EASE = [0.22, 0.7, 0.2, 1];
/// Keep the panel current while it sits on screen. The gate polls the broker on its
/// own schedule; this just re-reads what the host already knows.
const POLL_MS = 60_000;
const HOUR_MS = 60 * 60 * 1000;
const MINUTE_MS = 60 * 1000;
/// Only used before the first response lands. The window and the sampling resolution
/// are the server's to decide ([`crate::foundation::energy_history`]) and arrive with
/// the series — repeating them as constants here is how the axis ends up labelled 24h
/// while the payload holds something else.
const ASSUMED_WINDOW_HOURS = 24;
const ASSUMED_BUCKET_MINUTES = 10;

const T = {
  en: {
    title: "Your energy is used up",
    action: "Subscribe",
    account: "Account",
    thisDevice: "This device",
    // The broker's tier vocabulary (userbook: standard | pro | max). Standard is the
    // included allowance everyone gets, deliberately not called free.
    plans: { standard: "Standard", pro: "Pro", max: "Max" },
    planUnknown: "Plan",
    refresh: "Refresh",
    refreshing: "Refreshing…",
    remaining: "remaining",
    last24h: "Last 24 hours",
    noReadings: "No readings yet — this fills in while the app runs.",
    ago: (h) => `${h}h ago`,
    now: "now",
    resetsIn: "Resets in",
    resetsUnknown: "Reset time unknown",
    resettingNow: "resetting…",
    byok: "Running on your own API keys — there is no managed balance to show.",
  },
  zh: {
    title: "能量已经用完了",
    action: "订阅",
    account: "账号",
    thisDevice: "本机账号",
    plans: { standard: "标准版", pro: "专业版", max: "旗舰版" },
    planUnknown: "方案",
    refresh: "刷新",
    refreshing: "刷新中…",
    remaining: "剩余",
    last24h: "最近 24 小时",
    noReadings: "还没有记录 — 应用运行期间会逐步生成。",
    ago: (h) => `${h} 小时前`,
    now: "现在",
    resetsIn: "距离重置",
    resetsUnknown: "重置时间未知",
    resettingNow: "即将重置…",
    byok: "当前使用你自己的 API Key，没有可显示的额度。",
  },
};

function words() {
  const app = document.documentElement.lang || "";
  const chain = !app || /^system$/i.test(app)
    ? [navigator.language]
    : [app, navigator.language];
  for (const tag of chain) {
    if (/^zh\b/i.test(tag || "")) return T.zh;
    if (/^en\b/i.test(tag || "")) return T.en;
  }
  return T.en;
}
const L = words();

const num = (n) => (Number.isFinite(n) ? n.toLocaleString() : "—");

/// The countdown, as `H:MM:SS` — the one place a person can watch the wait shrink.
/// Empty string when there is no known reset (BYOK, or before the first poll).
function untilReset(resetsAt, nowMs) {
  const target = Date.parse(resetsAt || "");
  if (!Number.isFinite(target)) return null;
  const left = Math.max(0, target - nowMs);
  const total = Math.floor(left / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (v) => String(v).padStart(2, "0");
  return { left, text: h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}` };
}

/// Project the series into the 0–1000 × 0–200 viewBox and work out which stretches of
/// it were actually watched.
///
/// The curve is drawn as one unbroken shape. The level is a real quantity that existed
/// the whole time, and both ends of every unobserved stretch are known, so cutting the
/// line there says "no energy" when it means "no reading" — which reads as a broken
/// chart, not as information. Instead the stretch between two readings further apart
/// than `gapMs` is drawn dashed: the shape stays continuous, and the part we are
/// inferring rather than reporting is visibly marked as such.
function shape(points, ceiling, edgeMs, windowMs, gapMs) {
  const start = edgeMs - windowMs;
  const x = (at) => ((at - start) / windowMs) * 1000;
  const y = (remaining) => 200 - (Math.max(0, Math.min(remaining, ceiling)) / ceiling) * 200;
  const all = points.map((p) => ({ x: x(p.at), y: y(p.remaining), at: p.at }));
  const runs = [];
  const bridges = [];
  let run = [];
  for (const p of all) {
    const previous = run[run.length - 1];
    if (previous && p.at - previous.at > gapMs) {
      runs.push(run);
      bridges.push([previous, p]);
      run = [];
    }
    run.push(p);
  }
  if (run.length) runs.push(run);
  const area = all.length
    ? `${all[0].x},200 ${all.map((p) => `${p.x},${p.y}`).join(" ")} ${all[all.length - 1].x},200`
    : "";
  return { area, runs, bridges, count: all.length };
}

function EnergyChart({ points, ceiling, edgeMs, windowMs, gapMs }) {
  const { area, runs, bridges, count } = useMemo(
    () =>
      ceiling > 0
        ? shape(points, ceiling, edgeMs, windowMs, gapMs)
        : { area: "", runs: [], bridges: [], count: 0 },
    [points, ceiling, edgeMs, windowMs, gapMs],
  );
  if (count < 2) {
    return <p className="energy__empty">{L.noReadings}</p>;
  }
  return (
    <svg
      className="energy__chart"
      viewBox="0 0 1000 200"
      preserveAspectRatio="none"
      role="img"
      aria-label={L.last24h}
    >
      {[0.25, 0.5, 0.75].map((f) => (
        <line
          key={f}
          className="energy__grid"
          x1={f * 1000}
          x2={f * 1000}
          y1="0"
          y2="200"
          vectorEffect="non-scaling-stroke"
        />
      ))}
      <polygon className="energy__area" points={area} />
      {bridges.map(([from, to], i) => (
        <line
          key={`bridge-${i}`}
          className="energy__bridge"
          x1={from.x}
          y1={from.y}
          x2={to.x}
          y2={to.y}
          vectorEffect="non-scaling-stroke"
        />
      ))}
      {runs.map((run, i) => (
        <polyline
          key={`run-${i}`}
          className="energy__line"
          points={run.map((p) => `${p.x},${p.y}`).join(" ")}
          fill="none"
          vectorEffect="non-scaling-stroke"
        />
      ))}
      <line
        className="energy__floor"
        x1="0"
        x2="1000"
        y1="200"
        y2="200"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}

export default function OutOfEnergy() {
  const [href, setHref] = useState(FALLBACK_URL);
  const [energy, setEnergy] = useState(null);
  const [refreshing, setRefreshing] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const alive = useRef(true);

  useEffect(() => {
    // Set on entry, not only cleared on exit: a remount (StrictMode's double-invoke,
    // or the gate re-showing the view) must not leave the flag stuck false, which
    // would silently discard every later response.
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);

  const load = useCallback(async (force) => {
    if (force) setRefreshing(true);
    try {
      const response = await fetch(
        `/api/account/energy?history=true${force ? "&refresh=true" : ""}`,
      );
      const data = await response.json();
      if (alive.current && data && typeof data === "object") setEnergy(data);
    } catch {
      // Leave the last good reading on screen: a failed poll is not new information
      // about the account, and blanking the panel would look like the account itself
      // went away.
    } finally {
      if (alive.current) setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    load(false);
    const timer = setInterval(() => load(false), POLL_MS);
    return () => clearInterval(timer);
  }, [load]);

  // One second tick, for the countdown only.
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    fetch("/api/account/subscribe")
      .then((response) => response.json())
      .then((data) => {
        if (alive.current && typeof data?.url === "string" && data.url) {
          setHref(data.url);
        }
      })
      .catch(() => {});
  }, []);

  const managed = energy ? energy.managed !== false : true;
  const total = Number(energy?.total) || 0;
  const remaining = Math.max(0, Number(energy?.remaining) || 0);
  const history = energy?.history || null;
  // The ceiling the chart is drawn against: the largest ceiling seen in the window, so
  // a tier change mid-window doesn't rescale the past into a different shape.
  const ceiling = useMemo(() => {
    const seen = (history?.points || []).map((p) => p.total);
    return Math.max(total, ...seen, 0);
  }, [history, total]);
  const fraction = ceiling > 0 ? remaining / ceiling : 0;
  // The chart's right edge advances once a minute, not once a second: on a 24-hour
  // axis a finer edge is invisible, and it would re-project the series on every tick
  // of the countdown.
  const edgeMs = useMemo(() => Math.floor(now / 60_000) * 60_000, [now]);
  // The window and the resolution come from the payload, so the axis always says what
  // was actually served.
  const windowHours = Number(history?.window_hours) || ASSUMED_WINDOW_HOURS;
  const windowMs = windowHours * HOUR_MS;
  // Two buckets apart is still contiguous sampling; wider than that, nothing was
  // observed in between and the curve is drawn as inferred.
  const gapMs = (Number(history?.bucket_minutes) || ASSUMED_BUCKET_MINUTES) * 2 * MINUTE_MS;
  // The curve runs all the way to the right edge, because the level *now* is known —
  // it is the same number printed above the gauge. Without this the line stops at the
  // last recorded bucket and the chart looks like it lost the present.
  const series = useMemo(() => {
    const recorded = history?.points || [];
    const last = recorded[recorded.length - 1];
    if (!recorded.length || !energy) return recorded;
    return last.at >= edgeMs
      ? recorded
      : [...recorded, { at: edgeMs, remaining, total: last.total || total }];
  }, [history, energy, edgeMs, remaining, total]);
  const countdown = untilReset(energy?.resets_at, now);
  const plan = L.plans[(energy?.tier || "").toLowerCase()] || L.planUnknown;
  const who = energy?.name || energy?.email || (energy?.signed_in ? L.account : L.thisDevice);
  return (
    <motion.main
      className="energy"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.35, ease: EASE }}
      role="alert"
    >
      <style>{`
        .energy {
          box-sizing: border-box;
          width: 100%;
          height: 100%;
          min-height: 100%;
          display: flex;
          flex-direction: column;
          gap: clamp(18px, 3.4vh, 34px);
          padding:
            max(clamp(28px, 5vh, 56px), var(--hi-safe-top))
            max(clamp(24px, 4vw, 64px), var(--hi-safe-right))
            max(clamp(28px, 5vh, 56px), var(--hi-safe-bottom))
            max(clamp(24px, 4vw, 64px), var(--hi-safe-left));
          color: var(--fg);
          font-family: var(--font-display);
          background: var(--surface-strong);
        }
        .energy__row {
          display: flex;
          align-items: flex-start;
          justify-content: space-between;
          gap: 16px;
        }
        .energy__title {
          margin: 0;
          font-size: clamp(26px, 4.6vh, 40px);
          font-weight: 680;
          line-height: 1.1;
          letter-spacing: 0;
        }
        .energy__who {
          margin: 8px 0 0;
          display: flex;
          align-items: center;
          gap: 8px;
          min-width: 0;
          color: var(--fg-dim);
          font-size: 14px;
          letter-spacing: 0;
        }
        .energy__name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .energy__plan {
          flex: none;
          padding: 2px 8px;
          border: 1px solid var(--line-strong);
          border-radius: 999px;
          font-family: var(--font-mono);
          font-size: 11px;
          letter-spacing: 0;
        }
        .energy__aside {
          flex: none;
          display: flex;
          flex-direction: column;
          align-items: flex-end;
          gap: 10px;
        }
        .energy__countdown {
          margin: 0;
          display: flex;
          align-items: baseline;
          gap: 8px;
          color: var(--fg-dim);
          font-size: 12px;
          letter-spacing: 0;
        }
        .energy__clock {
          color: var(--fg);
          font-family: var(--font-mono);
          font-size: 20px;
          font-weight: 700;
          line-height: 1;
          letter-spacing: 0;
          font-variant-numeric: tabular-nums;
        }
        .energy__refresh {
          flex: none;
          display: inline-flex;
          align-items: center;
          gap: 6px;
          min-height: 32px;
          padding: 0 12px;
          border: 1px solid var(--line-strong);
          border-radius: 8px;
          color: var(--fg-dim);
          background: transparent;
          font-family: var(--font-display);
          font-size: 13px;
          letter-spacing: 0;
          cursor: pointer;
          transition: color 140ms ease, border-color 140ms ease;
        }
        .energy__refresh:hover:not(:disabled) { color: var(--fg); border-color: var(--fg-mute); }
        .energy__refresh:disabled { cursor: default; opacity: 0.6; }
        .energy__refresh svg { width: 13px; height: 13px; }
        .energy__spin { transform-origin: 50% 50%; animation: energy-spin 900ms linear infinite; }
        @keyframes energy-spin { to { transform: rotate(360deg); } }
        .energy__figure {
          display: flex;
          align-items: baseline;
          gap: 10px;
          font-family: var(--font-mono);
        }
        .energy__remaining {
          font-size: clamp(34px, 6vh, 54px);
          font-weight: 700;
          line-height: 1;
          letter-spacing: 0;
        }
        .energy__total { color: var(--fg-mute); font-size: 17px; letter-spacing: 0; }
        .energy__unit { color: var(--fg-dim); font-family: var(--font-display); font-size: 14px; }
        .energy__gauge {
          height: 8px;
          margin-top: 14px;
          overflow: hidden;
          border-radius: 999px;
          background: var(--line);
        }
        .energy__fill { height: 100%; border-radius: 999px; background: var(--accent); transition: width 400ms ease; }
        .energy__section {
          flex: 1 1 auto;
          min-height: 120px;
          display: flex;
          flex-direction: column;
          gap: 8px;
        }
        .energy__label { color: var(--fg-dim); font-size: 12px; letter-spacing: 0; }
        .energy__chart { flex: 1 1 auto; width: 100%; min-height: 0; display: block; }
        .energy__grid { stroke: var(--line); stroke-width: 1; }
        .energy__floor { stroke: var(--line-strong); stroke-width: 1; }
        .energy__area { fill: var(--accent); opacity: 0.16; }
        .energy__line { stroke: var(--accent); stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
        /* An unobserved stretch: same curve, drawn as inference rather than record. */
        .energy__bridge { stroke: var(--accent); stroke-width: 2; stroke-dasharray: 3 5; stroke-linecap: round; opacity: 0.5; }
        .energy__empty {
          flex: 1 1 auto;
          margin: 0;
          display: grid;
          place-items: center;
          border: 1px dashed var(--line-strong);
          border-radius: 10px;
          color: var(--fg-mute);
          font-size: 13px;
          line-height: 1.5;
          text-align: center;
        }
        .energy__axis {
          display: flex;
          justify-content: space-between;
          color: var(--fg-mute);
          font-family: var(--font-mono);
          font-size: 11px;
          letter-spacing: 0;
        }
        .energy__foot { display: flex; justify-content: flex-end; }
        .energy__action {
          flex: none;
          min-height: 46px;
          display: inline-flex;
          align-items: center;
          justify-content: center;
          box-sizing: border-box;
          padding: 0 26px;
          border: 1px solid #fd605e;
          border-radius: 8px;
          color: #fff;
          background: #fd605e;
          font-size: 15px;
          font-weight: 700;
          letter-spacing: 0;
          text-decoration: none;
          transition: background 160ms ease, border-color 160ms ease, box-shadow 160ms ease;
        }
        .energy__action:hover { border-color: #e74d4b; background: #e74d4b; }
        .energy__action:focus-visible {
          outline: none;
          box-shadow: 0 0 0 3px var(--surface-strong), 0 0 0 5px #fd605e;
        }
        .energy__byok { margin: 0; color: var(--fg-dim); font-size: 15px; line-height: 1.5; }
        @media (max-width: 520px) {
          .energy { gap: 18px; }
          /* Too narrow to keep a column of controls beside the title: the aside
             unstacks and sits under the account line as one row. */
          .energy__row { flex-direction: column; }
          .energy__aside { flex-direction: row-reverse; align-items: center; justify-content: flex-end; gap: 14px; }
          .energy__foot { flex-direction: column; align-items: stretch; }
          .energy__action { width: 100%; }
        }
      `}</style>

      <div className="energy__row">
        <div>
          <h1 className="energy__title">{L.title}</h1>
          <p className="energy__who">
            <span className="energy__name">{who}</span>
            <span className="energy__plan">{plan}</span>
          </p>
        </div>
        {/* The wait and the way to re-check it are the same question, so they sit
            together: what's left, and when it comes back. */}
        <div className="energy__aside">
          <button
            className="energy__refresh"
            onClick={() => load(true)}
            disabled={refreshing}
            type="button"
          >
            <svg viewBox="0 0 16 16" fill="none" aria-hidden>
              <g className={refreshing ? "energy__spin" : undefined}>
                <path
                  d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9"
                  stroke="currentColor"
                  strokeWidth="1.6"
                  strokeLinecap="round"
                />
                <path
                  d="M13.6 1.9v3.1h-3.1"
                  stroke="currentColor"
                  strokeWidth="1.6"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </g>
            </svg>
            {refreshing ? L.refreshing : L.refresh}
          </button>
          <p className="energy__countdown">
            <span>{countdown ? L.resetsIn : L.resetsUnknown}</span>
            {countdown && (
              <span className="energy__clock">
                {countdown.left > 0 ? countdown.text : L.resettingNow}
              </span>
            )}
          </p>
        </div>
      </div>

      {managed ? (
        <>
          <div>
            <div className="energy__figure">
              <span className="energy__remaining">{num(remaining)}</span>
              <span className="energy__total">/ {num(total)}</span>
              <span className="energy__unit">{L.remaining}</span>
            </div>
            <div className="energy__gauge">
              <div className="energy__fill" style={{ width: `${Math.round(fraction * 100)}%` }} />
            </div>
          </div>

          <div className="energy__section">
            <span className="energy__label">{L.last24h}</span>
            <EnergyChart
              points={series}
              ceiling={ceiling}
              edgeMs={edgeMs}
              windowMs={windowMs}
              gapMs={gapMs}
            />
            <div className="energy__axis">
              <span>{L.ago(windowHours)}</span>
              <span>{L.ago(Math.round(windowHours / 2))}</span>
              <span>{L.now}</span>
            </div>
          </div>
        </>
      ) : (
        <p className="energy__byok">{L.byok}</p>
      )}

      <div className="energy__foot">
        <a className="energy__action" href={href} target="_blank" rel="noopener noreferrer">
          {L.action}
        </a>
      </div>
    </motion.main>
  );
}
