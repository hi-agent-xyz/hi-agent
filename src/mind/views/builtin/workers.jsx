// purpose: 手头在跑的 — every agent session, live and just-ended, in the shape of who
// answers to whom. Read-only.
//
// Read-only on purpose: the registry has no stop verb, and a button that pretended to kill
// a worker would be worse than no button.
//
// Three things this surface has to get right, all of them lessons from gaps.md:
//
// 1. **Structure, because a flat list hid the delegation.** Every row was a sibling, so a
//    worker and the rung that spawned it read as peers and there was no way to see that
//    Cognition had three sessions out. Workers nest under their owner. One indent, rows not
//    columns — the frame here is the window *minus* the conversation rail
//    (`docs/arch/stage.md`, ~320-460px), so horizontal lanes would be unreadable at the
//    width this actually renders at.
// 2. **"Ended" is an answer, not an absence.** The switchboard is live-by-construction — an
//    entry exists between register and unregister, and a finished worker is simply gone — so
//    a watch that died thirty seconds ago looked identical to one that never existed. That is
//    §2: the agent said "挂着呢,一直在盯" while the page showed nothing. Ended sessions are
//    listed by recency, and one the process died underneath says so.
// 3. **Doing, not just said.** `tail` is the session's own words, and a worker grinding
//    through shell commands says nothing for minutes — so a blank row read as a dead one.
//    `doing` is the other half, and the two never share a line.
//
// Clicking any row — live or ended — opens its verbatim wire log. Those frames have been
// written for every session all along (`WireTap::with_durable_log`) and nothing could read
// them back: the path is keyed by (run, session), ids restart at 1 each boot, and an ended
// session was gone from the roster that knew its id. This is the reader.
//
// Colour comes from the host theme tokens (see tasks.jsx for the vocabulary). Polls, because
// the whole value is that it is current.
import { useState, useEffect, useCallback, useRef, useMemo } from "react";

const api = {
  list: () => fetch("/api/workers").then((r) => r.json()),
  ended: () => fetch("/api/workers/ended?limit=40").then((r) => r.json()),
  frames: (id, run) => {
    const q = run ? `?run=${encodeURIComponent(run)}` : "";
    return fetch(`/api/workers/${encodeURIComponent(id)}/frames${q}`).then((r) => r.json());
  },
};

/** How often the roster re-reads. Both endpoints are in-memory reads on the server — the
 *  live roster is a lock on a HashMap, the ended list a capped Vec — so this is cheap
 *  enough to be genuinely live rather than nearly-live. Held off while the page is hidden,
 *  since nothing is being read then. */
const POLL_MS = 2000;

// ── words ─────────────────────────────────────────────────────────────────────
// English is the default and the fallback. `Worker` is this system's own vocabulary —
// the rung a session runs at — so the title stays the English word in both. The rung
// labels under `role` are descriptions of what each one does, so those do get said in
// the reader's language.
//
// The registry stores the **role**, and a worker's role carries its type, so a row can
// say "做界面" rather than only "干活的".
//
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped
// here. Until that exists, an unsupported language lands on English.
const T = {
  en: {
    title: "Workers",
    runningN: (n) => `${n} running`,
    endedN: (n) => `${n} ended`,
    emptyBig: "Nothing is running right now.",
    emptySub: "That's the whole answer — not a page still loading.",
    // Short on purpose: these render inside a small pill beside the task, and the
    // Chinese equivalents are two or three characters. A clause does not fit.
    role: {
      worker: "working",
      cognition: "thinking",
      deliberation: "mulling",
      reflection: "filing",
      reaction: "speaking",
    },
    // `general` is deliberately absent: a general worker is just "working", which the
    // role label already says, and a second word for it would only add noise to the
    // common row.
    type: {
      "view-builder": "building a view",
      "view-reviewer": "reviewing a view",
      "decision-maker": "deciding",
      "file-filer": "filing a file",
    },
    noTask: "(no note on what it's doing)",
    busy: "running", idle: "idle",
    turns: (n) => `${n} turns`, queued: "mail waiting",
    live: "Live", endedHead: "Just ended",
    // An owner that shut down while its worker kept running. The condition behind the
    // dropped report, so it is named rather than shown as a row with no parent.
    orphans: "Still running, owner gone",
    orphanNote: (id) => `owner ${id} is no longer registered`,
    // On an ended row the tree is gone, so the owner is named by id — the only address
    // there is, and the one a reader can match against another row on the page.
    ownedBy: (id) => `owned by ${id}`,
    endedAgo: (t) => `ended ${t} ago`,
    lostToRestart: "lost to a restart",
    lostNote: "The process ended underneath it — nothing got to report what it found.",
    ranFor: (t) => `ran ${t}`,
    noEnded: "No sessions have ended yet.",
    // The frame log panel.
    frames: "Wire log",
    framesOf: (n) => `${n} frames`,
    framesTruncated: (shown, total) => `last ${shown} of ${total}`,
    noFrames: "No frames were kept for this session.",
    noFramesNote: "Nothing crossed the wire under this id, or its run's files are gone.",
    framesFailed: "That session's wire log could not be read.",
    loading: "Reading…",
    close: "Close",
    send: "sent", recv: "received", stderr: "stderr",
    runLabel: (r) => `run ${r}`,
    sessionLabel: (id) => `session ${id}`,
    secs: (n) => `${n}s`, mins: (n) => `${n}m`, hours: (n) => `${n}h`, days: (n) => `${n}d`,
  },
  zh: {
    title: "Workers",
    runningN: (n) => `${n} 个在跑`,
    endedN: (n) => `${n} 个结束了`,
    emptyBig: "现在没有活在跑。",
    emptySub: "这就是全部 —— 不是还没加载出来。",
    role: {
      worker: "干活的",
      cognition: "想事情的",
      deliberation: "琢磨的",
      reflection: "整理记忆的",
      reaction: "说话的",
    },
    type: {
      "view-builder": "做界面",
      "view-reviewer": "看界面",
      "decision-maker": "拿主意",
      "file-filer": "归档",
    },
    noTask: "（没写在做什么）",
    busy: "在跑", idle: "闲着",
    turns: (n) => `${n} 轮`, queued: "还有没读的",
    live: "在跑", endedHead: "刚结束",
    orphans: "还在跑,派它的已经没了",
    orphanNote: (id) => `派它的 ${id} 已经不在了`,
    ownedBy: (id) => `归 ${id}`,
    endedAgo: (t) => `${t}前结束`,
    lostToRestart: "重启时丢了",
    lostNote: "进程在它下面结束了 —— 它做了什么没来得及报。",
    ranFor: (t) => `跑了 ${t}`,
    noEnded: "还没有结束的会话。",
    frames: "原始帧",
    framesOf: (n) => `${n} 帧`,
    framesTruncated: (shown, total) => `共 ${total} 帧,这是最后 ${shown} 帧`,
    noFrames: "这个会话没留下帧。",
    noFramesNote: "这个 id 下没有东西走过线,或者那次运行的文件已经没了。",
    framesFailed: "读不到这个会话的原始帧。",
    loading: "读取中…",
    close: "关闭",
    send: "发出", recv: "收到", stderr: "标准错误",
    runLabel: (r) => `运行 ${r}`,
    sessionLabel: (id) => `会话 ${id}`,
    secs: (n) => `${n} 秒`, mins: (n) => `${n} 分钟`, hours: (n) => `${n} 小时`, days: (n) => `${n} 天`,
  },
};

// App setting first — the host puts it on `<html lang>` — then the system locale when
// that setting says to follow the person, then English.
function words() {
  const app = document.documentElement.lang || "";
  const chain = !app || /^system$/i.test(app) ? [navigator.language] : [app, navigator.language];
  for (const tag of chain) {
    if (/^zh\b/i.test(tag || "")) return T.zh;
    if (/^en\b/i.test(tag || "")) return T.en;
  }
  return T.en;
}
const L = words();

/** The ladder, top to bottom — the order `docs/arch/agents.md` gives: the voice, the
 *  thinking behind it, the outward brain, the housekeeper. A role not named here sorts
 *  after these rather than being dropped, so a sixth rung appears instead of vanishing. */
const LADDER = ["reaction", "deliberation", "cognition", "reflection"];

function label(row) {
  return L.type?.[row.type] || L.role[row.role] || row.role;
}

export default function Workers() {
  const [live, setLive] = useState(null);
  const [ended, setEnded] = useState([]);
  // The open row is held by its **address** — `{ id, run }` — not by the row object,
  // because the poll below replaces every row on each tick and a held object would freeze
  // the panel on the version that was clicked. A live row passes no run; the endpoint
  // defaults to the current one, which is the only run a live session can be in.
  const [open, setOpen] = useState(null);

  const reload = useCallback(async () => {
    const [a, b] = await Promise.all([
      api.list().catch(() => null),
      api.ended().catch(() => null),
    ]);
    // A failed fetch keeps the last good roster rather than blanking it. "Nothing is
    // running" is a load-bearing answer on this page, so it must never be produced by a
    // momentary 500.
    setLive((prev) => (a ? a.workers || [] : prev === null ? [] : prev));
    if (b) setEnded(b.ended || []);
  }, []);

  useEffect(() => {
    reload();
    const t = setInterval(() => {
      if (!document.hidden) reload();
    }, POLL_MS);
    // Re-read on the way back rather than showing however stale the last tick was.
    const onShow = () => { if (!document.hidden) reload(); };
    document.addEventListener("visibilitychange", onShow);
    return () => {
      clearInterval(t);
      document.removeEventListener("visibilitychange", onShow);
    };
  }, [reload]);

  const groups = useMemo(() => tree(live || []), [live]);

  if (live === null) {
    return (
      <div className="hi-workers">
        <style>{CSS}</style>
        <div className="hi-workers__scroll">
          <header className="hi-workers__head"><h1 className="hi-workers__h1">{L.title}</h1></header>
        </div>
      </div>
    );
  }

  const running = live.length;

  return (
    <div className="hi-workers">
      <style>{CSS}</style>

      <div className="hi-workers__scroll">
        <header className="hi-workers__head">
          <h1 className="hi-workers__h1">{L.title}</h1>
          <span className="hi-workers__count">
            {[running ? L.runningN(running) : null, ended.length ? L.endedN(ended.length) : null]
              .filter(Boolean)
              .join(" · ")}
          </span>
        </header>

        {running === 0 ? (
          <div className="hi-workers__empty">
            <div className="hi-workers__empty-big">{L.emptyBig}</div>
            <div className="hi-workers__empty-sub">{L.emptySub}</div>
          </div>
        ) : (
          <section className="hi-workers__section">
            <h2 className="hi-workers__section-head">{L.live}</h2>
            {groups.map((g) => (
              <div className="hi-workers__group" key={g.key}>
                {g.orphaned && <div className="hi-workers__group-note">{L.orphans}</div>}
                {g.owner && <LiveRow row={g.owner} open={open} setOpen={setOpen} />}
                {g.children.length > 0 && (
                  <div className="hi-workers__kids" data-rooted={g.owner ? "true" : undefined}>
                    {g.children.map((c) => (
                      <LiveRow key={c.id} row={c} open={open} setOpen={setOpen} indent />
                    ))}
                  </div>
                )}
              </div>
            ))}
          </section>
        )}

        <section className="hi-workers__section">
          <h2 className="hi-workers__section-head">{L.endedHead}</h2>
          {ended.length === 0 ? (
            <div className="hi-workers__none">{L.noEnded}</div>
          ) : (
            ended.map((e) => (
              <EndedRow key={`${e.run}:${e.session}`} row={e} open={open} setOpen={setOpen} />
            ))
          )}
        </section>
      </div>

      {open && <Frames addr={open} onClose={() => setOpen(null)} />}
    </div>
  );
}

/** Group the live roster into owner-with-children, in ladder order.
 *
 *  Keyed on the owner **id**, which is why the endpoint reports one. It used to report the
 *  owner's role *word* while the owner was live and its bare id only once the owner had
 *  died — so the id, the one thing a tree can be built from, was there exactly when it was
 *  useless. That the word worked at all was a coincidence of one session per rung being
 *  live at a time; it is a label, not an address.
 *
 *  Three kinds of group come out, and the third is the one worth the code: a rung with its
 *  workers · a rung with none · workers whose owner is **not in the roster at all**, which
 *  means the owner shut down while they kept running. Those get their own labelled group,
 *  because an orphan silently reparented to the root is the dropped-report condition made
 *  invisible all over again.
 */
function tree(rows) {
  const byId = new Map(rows.map((r) => [r.id, r]));
  const kids = new Map();
  const roots = [];
  const orphans = [];

  for (const r of rows) {
    if (!r.owner) {
      roots.push(r);
    } else if (byId.has(r.owner)) {
      if (!kids.has(r.owner)) kids.set(r.owner, []);
      kids.get(r.owner).push(r);
    } else {
      // Names an owner the roster does not have. Not a root — an orphan.
      orphans.push(r);
    }
  }

  const rank = (r) => {
    const i = LADDER.indexOf(r.role);
    return i === -1 ? LADDER.length : i;
  };
  const byStart = (a, b) => String(a.started).localeCompare(String(b.started));
  // Ladder first, then oldest first inside a rank — a session's place must not jump around
  // between two-second polls.
  roots.sort((a, b) => rank(a) - rank(b) || byStart(a, b));
  for (const list of kids.values()) list.sort(byStart);

  const groups = roots.map((r) => ({ key: r.id, owner: r, children: kids.get(r.id) || [] }));
  if (orphans.length) {
    orphans.sort(byStart);
    groups.push({ key: "__orphans__", owner: null, children: orphans, orphaned: true });
  }
  return groups;
}

function LiveRow({ row, open, setOpen, indent }) {
  // A live row is addressed by id with no run, so it cannot collide with an ended row that
  // happens to share the number — which, across runs, they routinely do.
  const isOpen = !!open && open.id === row.id && !open.run;
  return (
    <button
      type="button"
      className="hi-workers__row"
      data-indent={indent ? "true" : undefined}
      data-open={isOpen ? "true" : undefined}
      aria-expanded={isOpen}
      onClick={() => setOpen(isOpen ? null : { id: row.id, run: null })}
    >
      <span className="hi-workers__line">
        <span className={`hi-workers__dot${row.busy ? " is-busy" : ""}`} aria-hidden />
        <span className="hi-workers__pill">{label(row)}</span>
        <span className="hi-workers__task">{row.task || L.noTask}</span>
        <span className="hi-workers__elapsed">{elapsed(row.started)}</span>
      </span>

      <span className="hi-workers__meta">
        <span>{row.busy ? L.busy : L.idle}</span>
        {typeof row.turns === "number" && <span>{L.turns(row.turns)}</span>}
        {/* `queued` is a bool: the registry merges a burst into one prompt, so it knows
            there is mail waiting but never how much. */}
        {row.queued && <span className="is-accent">{L.queued}</span>}
        {/* Only once the owner has gone. While it is live the indent already shows who it
            is, and repeating it on every child is noise. */}
        {row.owner && !row.owner_role && (
          <span className="is-warn">{L.orphanNote(row.owner)}</span>
        )}
      </span>

      {/* Doing and said are different questions and never share a line. A row with a
          `doing` and no `tail` is a session working in silence — which is exactly what used
          to be indistinguishable from a dead one. */}
      {row.doing && <span className="hi-workers__doing">{row.doing}</span>}
      {row.tail && <span className="hi-workers__tail">{row.tail}</span>}
    </button>
  );
}

function EndedRow({ row, open, setOpen }) {
  const lost = row.how === "restart";
  const isOpen = !!open && open.id === String(row.session) && open.run === row.run;
  // A restart row has no end — nothing recorded one — so it is dated by its start.
  const when = row.ended || row.started;
  return (
    <button
      type="button"
      className="hi-workers__row is-ended"
      data-lost={lost ? "true" : undefined}
      data-open={isOpen ? "true" : undefined}
      aria-expanded={isOpen}
      onClick={() => setOpen(isOpen ? null : { id: String(row.session), run: row.run })}
    >
      <span className="hi-workers__line">
        <span className="hi-workers__dot" aria-hidden />
        <span className="hi-workers__pill">{label(row)}</span>
        <span className="hi-workers__task">{row.task || L.noTask}</span>
        <span className="hi-workers__elapsed">
          {lost ? L.lostToRestart : when ? L.endedAgo(elapsed(when)) : ""}
        </span>
      </span>

      <span className="hi-workers__meta">
        {typeof row.turns === "number" && <span>{L.turns(row.turns)}</span>}
        {row.started && row.ended && <span>{L.ranFor(between(row.started, row.ended))}</span>}
        {row.owner && <span>{L.ownedBy(row.owner)}</span>}
      </span>

      {/* Said plainly, because "it was cut off" and "it finished" are not the same outcome,
          and the difference is the reason this row exists at all. */}
      {lost && <span className="hi-workers__lost">{L.lostNote}</span>}
    </button>
  );
}

/** The zoom-in: one session's verbatim wire log.
 *
 *  Every JSON-RPC line that crossed to or from that session's subprocess, in order,
 *  uninterpreted. A frame the server could not parse arrives as a string and is shown as
 *  one — this is the record, so the frame nobody can read is exactly the frame someone came
 *  looking for. */
function Frames({ addr, onClose }) {
  const [state, setState] = useState({ status: "loading" });
  const [rawOf, setRawOf] = useState(null);
  const panel = useRef(null);

  useEffect(() => {
    panel.current?.focus();
    const onKey = (e) => { if (e.key === "Escape") onClose(); };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    let alive = true;
    setState({ status: "loading" });
    setRawOf(null);
    api.frames(addr.id, addr.run)
      .then((d) => alive && setState({ status: "ok", ...d }))
      .catch(() => alive && setState({ status: "failed" }));
    return () => { alive = false; };
  }, [addr.id, addr.run]);

  const frames = state.frames || [];

  return (
    <div className="hi-workers__scrim" onClick={onClose}>
      <div
        ref={panel}
        className="hi-workers__panel"
        role="dialog"
        aria-modal="true"
        aria-label={L.frames}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="hi-workers__panel-head">
          <div className="hi-workers__panel-title">
            <strong>{L.frames}</strong>
            <span className="hi-workers__panel-sub">
              {[
                L.sessionLabel(addr.id),
                state.run ? L.runLabel(state.run) : null,
                state.status === "ok" && frames.length
                  ? state.truncated
                    ? L.framesTruncated(frames.length, state.total)
                    : L.framesOf(state.total)
                  : null,
              ]
                .filter(Boolean)
                .join(" · ")}
            </span>
          </div>
          <button type="button" className="hi-workers__close" aria-label={L.close} onClick={onClose}>
            ×
          </button>
        </div>

        <div className="hi-workers__panel-body">
          {state.status === "loading" && <div className="hi-workers__none">{L.loading}</div>}
          {state.status === "failed" && <div className="hi-workers__none">{L.framesFailed}</div>}
          {state.status === "ok" && frames.length === 0 && (
            <div className="hi-workers__empty">
              <div className="hi-workers__empty-big">{L.noFrames}</div>
              <div className="hi-workers__empty-sub">{L.noFramesNote}</div>
            </div>
          )}
          {frames.map((f, i) => (
            <Frame
              key={i}
              frame={f}
              open={rawOf === i}
              onToggle={() => setRawOf(rawOf === i ? null : i)}
            />
          ))}
        </div>
      </div>
    </div>
  );
}

const DIR = { send: L.send, recv: L.recv, stderr: L.stderr };

function Frame({ frame, open, onToggle }) {
  // A frame the server could not parse comes through as a bare string. It still renders, as
  // itself, because dropping it is the one thing a verbatim log may not do.
  if (typeof frame === "string") {
    return <pre className="hi-workers__raw is-unparsed">{frame}</pre>;
  }
  const raw = typeof frame.raw === "string" ? frame.raw : JSON.stringify(frame);
  return (
    <div className="hi-workers__frame" data-dir={frame.dir || undefined}>
      <button type="button" className="hi-workers__frame-head" aria-expanded={open} onClick={onToggle}>
        <span className="hi-workers__frame-time">{clock(frame.ts)}</span>
        <span className="hi-workers__frame-dir">{DIR[frame.dir] || frame.dir || ""}</span>
        <span className="hi-workers__frame-method">{frame.method || "—"}</span>
        {typeof frame.seq === "number" && <span className="hi-workers__frame-seq">#{frame.seq}</span>}
      </button>
      {open ? (
        <pre className="hi-workers__raw">{pretty(raw)}</pre>
      ) : (
        <div className="hi-workers__frame-peek">{raw}</div>
      )}
    </div>
  );
}

/** Re-indent a raw line when it is JSON, and leave it exactly as it arrived when it is not
 *  — a stderr line is not JSON and must not be mangled into looking like it. */
function pretty(raw) {
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
}

function clock(ts) {
  if (!ts) return "";
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleTimeString(undefined, { hour12: false });
}

function elapsed(started) {
  if (!started) return "";
  const t = new Date(started).getTime();
  if (Number.isNaN(t)) return "";
  return span(Math.max(0, Math.floor((Date.now() - t) / 1000)));
}

function between(a, b) {
  const from = new Date(a).getTime();
  const to = new Date(b).getTime();
  if (Number.isNaN(from) || Number.isNaN(to)) return "";
  return span(Math.max(0, Math.floor((to - from) / 1000)));
}

function span(s) {
  if (s < 60) return L.secs(s);
  if (s < 3600) return L.mins(Math.floor(s / 60));
  if (s < 86400) return L.hours(Math.floor(s / 3600));
  return L.days(Math.floor(s / 86400));
}

const CSS = `
  .hi-workers,
  .hi-workers * {
    box-sizing: border-box;
  }

  /* No ground of its own: the page stands on the layer's paper, which runs under the safe
     padding to the window edge. A flat colour here would fill only the padded content box
     and frame the page in the paper it doesn't match. */
  .hi-workers {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
    color: var(--fg);
    font-family: var(--font-display);
    --w-shadow: 0 1px 2px var(--shadow), 0 8px 22px var(--shadow);
    --w-mono: ui-monospace, SFMono-Regular, Menlo, monospace;
  }

  /* The scroller is a child, not the root: the root stays the positioning context for the
     panel below, and a scrolling ancestor would re-anchor it. The bottom padding is the
     strip the caption pill rises through — deliberately unpadded by the host, since
     reserving it would cost every view a slice of frame. */
  .hi-workers__scroll {
    width: 100%;
    height: 100%;
    min-height: 0;
    overflow-y: auto;
    padding: 28px clamp(16px, 3vw, 44px) 128px;
  }

  /* Wrapped in :where() so the reset carries **zero** specificity. Written bare, this
     selector is (0,1,1) — one class plus a type — which outranks every single-class rule
     that styles a button here, and a reset that beats the components it exists to prepare
     is not a reset. It silently ate the row's padding and surface (rows rendered as bare
     text under a floating shadow, since box-shadow is the one thing it doesn't set), the
     close button's centring, and the frame head's monospace, via `font: inherit`. It still
     beats the UA sheet, which loses to any author rule regardless of specificity. */
  :where(.hi-workers button) {
    appearance: none;
    border: none;
    background: none;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
    padding: 0;
  }

  .hi-workers button:focus-visible {
    outline: 3px solid var(--accent-soft);
    outline-offset: 2px;
  }

  .hi-workers__head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 18px;
  }

  .hi-workers__h1 {
    margin: 0;
    font-size: 30px;
    font-weight: 800;
  }

  .hi-workers__count {
    font-size: 13px;
    font-weight: 600;
    color: var(--fg-mute);
    text-align: right;
  }

  .hi-workers__section + .hi-workers__section {
    margin-top: 26px;
  }

  .hi-workers__section-head {
    margin: 0 0 10px;
    font-size: 11.5px;
    font-weight: 700;
    letter-spacing: .08em;
    text-transform: uppercase;
    color: var(--fg-mute);
  }

  .hi-workers__group + .hi-workers__group {
    margin-top: 10px;
  }

  .hi-workers__group-note {
    padding: 0 2px 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--danger);
  }

  /* One indent, and a rule to carry the eye from an owner down to what it spawned. The
     frame here can be the window minus a ~400px conversation rail, so the inset stays small
     and the nesting is one level only — depth would cost width the task line needs. */
  .hi-workers__kids {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 6px;
  }

  .hi-workers__kids[data-rooted="true"] {
    margin-left: 14px;
    padding-left: 14px;
    border-left: 2px solid var(--line);
  }

  .hi-workers__row {
    display: block;
    width: 100%;
    padding: 12px 14px;
    background: var(--surface-strong);
    border-radius: 14px;
    box-shadow: var(--w-shadow);
    transition: background 120ms var(--ease, ease);
  }

  .hi-workers__row:hover {
    background: var(--surface);
  }

  .hi-workers__row[data-open="true"] {
    box-shadow: 0 0 0 2px var(--accent-line), var(--w-shadow);
  }

  .hi-workers__row[data-indent="true"] {
    border-radius: 12px;
  }

  /* An ended row is quieter than a live one by construction — flat, outlined, no lift. The
     page should read live-first without needing a legend to say so. */
  .hi-workers__row.is-ended {
    background: var(--surface);
    border: 1px solid var(--line);
    box-shadow: none;
  }

  .hi-workers__row.is-ended + .hi-workers__row.is-ended {
    margin-top: 6px;
  }

  .hi-workers__row.is-ended[data-lost="true"] {
    border-color: var(--danger-line);
    background: var(--danger-wash);
  }

  .hi-workers__line {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }

  .hi-workers__dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--line-strong);
  }

  .hi-workers__dot.is-busy {
    background: var(--accent);
  }

  .hi-workers__pill {
    flex: none;
    padding: 3px 9px;
    border-radius: 999px;
    font-size: 11.5px;
    font-weight: 700;
    color: var(--accent);
    background: var(--accent-wash);
  }

  .is-ended .hi-workers__pill {
    color: var(--fg-mute);
    background: transparent;
    border: 1px solid var(--line);
  }

  /* min-width:0 so the ellipsis can happen at all — a flex child sizes to its content by
     default and would push the elapsed time off the row instead of truncating. */
  .hi-workers__task {
    flex: 1;
    min-width: 0;
    font-size: 14.5px;
    font-weight: 620;
    letter-spacing: -.01em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hi-workers__elapsed {
    flex: none;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--fg-mute);
  }

  .hi-workers__meta {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    margin-top: 7px;
    font-size: 12px;
    color: var(--fg-mute);
  }

  .hi-workers__meta .is-accent {
    color: var(--accent);
    font-weight: 600;
  }

  .hi-workers__meta .is-warn {
    color: var(--danger);
    font-weight: 600;
  }

  /* Monospace, because it is nearly always a command or a tool name and proportional type
     makes those hard to scan. One line: the panel is where the whole of it lives. */
  .hi-workers__doing {
    display: block;
    margin-top: 8px;
    font-family: var(--w-mono);
    font-size: 11.5px;
    color: var(--accent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hi-workers__tail {
    display: block;
    margin-top: 6px;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--fg-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hi-workers__lost {
    display: block;
    margin-top: 7px;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--danger);
  }

  .hi-workers__empty {
    padding: 40px 8px;
    text-align: center;
  }

  .hi-workers__empty-big {
    font-size: 17px;
    font-weight: 600;
    color: var(--fg-dim);
  }

  .hi-workers__empty-sub {
    margin-top: 7px;
    font-size: 13.5px;
    color: var(--fg-mute);
  }

  .hi-workers__none {
    padding: 14px 2px;
    font-size: 13px;
    color: var(--fg-mute);
  }

  /* ── the wire log panel ───────────────────────────────────────────────────── */

  /* Absolute, not fixed: it belongs to this view's frame. Fixed would break out of the
     content inset and cover the conversation rail, which the plane model says an agent
     surface may never do (docs/arch/stage.md). */
  .hi-workers__scrim {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: clamp(12px, 3vh, 40px) clamp(12px, 3vw, 40px) 128px;
    background: color-mix(in srgb, var(--bg-0) 62%, transparent);
    backdrop-filter: blur(3px);
  }

  .hi-workers__panel {
    display: flex;
    flex-direction: column;
    width: min(920px, 100%);
    max-height: 100%;
    min-height: 0;
    overflow: hidden;
    background: var(--surface-strong);
    border-radius: 18px;
    box-shadow: var(--shadow-strong);
  }

  .hi-workers__panel-head {
    flex: none;
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--line);
  }

  .hi-workers__panel-title {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 14px;
  }

  .hi-workers__panel-sub {
    font-size: 12px;
    color: var(--fg-mute);
  }

  .hi-workers__close {
    flex: none;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    font-size: 19px;
    line-height: 1;
    text-align: center;
    color: var(--fg-mute);
  }

  .hi-workers__close:hover {
    background: var(--accent-wash);
    color: var(--accent);
  }

  .hi-workers__panel-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 10px 12px 16px;
  }

  .hi-workers__frame {
    padding: 7px 4px;
    border-bottom: 1px solid var(--line);
  }

  .hi-workers__frame-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
    width: 100%;
    font-family: var(--w-mono);
    font-size: 11.5px;
  }

  .hi-workers__frame-time {
    flex: none;
    color: var(--fg-mute);
  }

  /* Direction is the first thing you look for in a wire log, so it carries the colour
     rather than an arrow glyph, which reads as decoration at this size. */
  .hi-workers__frame-dir {
    flex: none;
    width: 62px;
    font-weight: 700;
    color: var(--fg-mute);
  }

  .hi-workers__frame[data-dir="send"] .hi-workers__frame-dir {
    color: var(--accent);
  }

  .hi-workers__frame[data-dir="recv"] .hi-workers__frame-dir {
    color: var(--accent-2);
  }

  .hi-workers__frame[data-dir="stderr"] .hi-workers__frame-dir {
    color: var(--danger);
  }

  .hi-workers__frame-method {
    flex: 1;
    min-width: 0;
    font-weight: 600;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hi-workers__frame-seq {
    flex: none;
    color: var(--fg-mute);
  }

  .hi-workers__frame-peek {
    margin-top: 4px;
    padding-left: 2px;
    font-family: var(--w-mono);
    font-size: 11px;
    line-height: 1.5;
    color: var(--fg-mute);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hi-workers__raw {
    margin: 6px 0 2px;
    padding: 10px 12px;
    max-height: 40vh;
    overflow: auto;
    background: var(--bg-0);
    border-radius: 10px;
    font-family: var(--w-mono);
    font-size: 11px;
    line-height: 1.55;
    color: var(--fg-dim);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .hi-workers__raw.is-unparsed {
    color: var(--danger);
  }
`;
