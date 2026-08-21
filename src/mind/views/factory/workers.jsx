// purpose: 手头在跑的 — every agent session, live and just-ended, in the shape of who
// answers to whom. Read-only.
//
// Read-only on purpose: the registry has no stop verb, and a button that pretended to kill
// a worker would be worse than no button.
//
// Five things this surface has to get right, all of them lessons from gaps.md:
//
// 1. **Structure, because a flat list hid the delegation.** Every row was a sibling, so a
//    worker and the rung that spawned it read as peers and there was no way to see that
//    Cognition had three sessions out.
//
//    That structure is now **drawn as the tree it is**: roots on top, what they spawned
//    below them, an arrow from each owner down to each session it created. It is a real
//    tree and not one indent's worth of nesting — a worker that creates a worker is
//    a third row, and the picture says so.
//
//    It needs a layout engine, because ownership is data and no CSS box model lays out a
//    tree of unequal subtrees. `layout()` is that engine: a tidy pass in the shape of
//    Reingold–Tilford — bottom-up subtree widths, then a top-down placement that centres
//    each owner over the span of its children — against measured card heights, one row per
//    depth. It returns nothing but geometry, and everything on the canvas (card position,
//    every arrow endpoint) is read off it, so a card and its arrow cannot disagree.
//
//    The cards are the width the engine gives them (`NODE_W`), the same for every session.
//    A card gives each field its own line and lets the ones that are prose wrap; the old
//    full-width row spent the frame on a mostly-empty line and then bought the title out
//    of what was left, which is how a worker's task became four words and an ellipsis.
//
//    **And a card's title is one line, because what the server sends is now a headline.**
//    A session used to be registered under the brief it was handed — for real work, a
//    paragraph — so this page could only ever show its opening clause, which is setup and
//    never the subject. `create_worker` takes a written `title` beside the `task` now
//    (`Status::title`), so the line on the card is the line someone wrote to be read, and
//    the brief lives where a brief belongs: the session's first prompt, whole, one click
//    away under "What happened".
// 2. **"Ended" is an answer, not an absence.** The switchboard is live-by-construction — an
//    entry exists between register and unregister, and a finished worker is simply gone — so
//    a watch that died thirty seconds ago looked identical to one that never existed. That is
//    §2: the agent said "挂着呢,一直在盯" while the page showed nothing. Ended sessions are
//    listed by recency, and one the process died underneath says so.
//
//    They sit in a band under the tree rather than in a column beside it, and the band is
//    two cards deep and scrolls. A tree is as wide as the delegation happens to be; a column
//    beside it would be squeezed by exactly the thing this page exists to show.
// 3. **Doing, not just said.** `tail` is the session's own words, and a worker grinding
//    through shell commands says nothing for minutes — so a blank row read as a dead one.
//    `doing` is the other half, and the two never share a line.
// 4. **A card has one state, and which fields are meaningful is a function of it.** The
//    three above are each a field earning its place; this one is about what happens when
//    several of them are drawn as though they were independent. They were, and the card
//    contradicted itself three separate times — see `LiveCard`. The server now sends one
//    `state` word and this file gates on it.
// 5. **Quiet is two different endings, and the state word says neither.** `idle` is what a
//    worker between instructions reports and what a worker whose turn died reports, because
//    the word is folded from busy/queued and the ending was nowhere on the wire. On
//    2026-08-18 three workers failed on a 429 inside two minutes, drew three `idle 3m` cards,
//    and were sent "Continue now; do not leave this idle" — a recovery aimed at laziness.
//    `last_turn` is the ending, drawn on quiet cards only, and only when it is bad news.
//
// Clicking any card — live or ended — opens what that session did. Those frames have been
// written for every session all along (`WireTap::with_durable_log`) and nothing could read
// them back: the path is keyed by (run, session), ids restart at 1 each boot, and an ended
// session was gone from the roster that knew its id. This is the reader.
//
// It reads the log **folded into messages**, not frame by frame. The record is verbatim and
// stays that way, but a record is not a reading of itself: one sentence the agent said
// crosses the wire as an `item/started`, hundreds of deltas and an `item/completed`, so
// row-per-frame is a wall of fragments — 11,891 frames on the logs this was measured
// against, for 369 things that actually happened. The fold is the server's
// (`foundation::codex::messages`, `GET /api/workers/{id}/messages`) rather than this file's,
// so `curl` during a journey test sees exactly what the page sees. The verbatim log stays
// one click away, because it is what the fold can be wrong about.
//
// **Nothing on the canvas is positioned by CSS, so nothing on it may be animated by CSS
// either.** Cards are absolutely placed at the engine's coordinates and the arrows are one
// SVG path per edge in the same coordinate space, which means a card that glides to a new
// position while its arrow snaps is not a rough edge — it is the arrow detaching from the
// card. So one animator (`useCanvas`) owns both: it eases every card's position and every
// session's fade in one rAF pass and writes them straight to the DOM, recomputing each
// arrow from the *current* card positions rather than the target ones. It runs only while
// something is actually moving. A session appearing and a session ending are the two
// moments this page is watched for, so they are the two it animates.
//
// Colour comes from the host theme tokens (see tasks.jsx for the vocabulary). Polls, because
// the whole value is that it is current.
import { useState, useEffect, useCallback, useRef, useMemo, useReducer } from "react";

const api = {
  list: () => fetch("/api/workers").then((r) => r.json()),
  ended: () => fetch("/api/workers/ended?limit=40").then((r) => r.json()),
  // The two readings of one session's log. `messages` is the fold — what happened —
  // and `frames` is the record it was folded from. Same file, same address, and the
  // folding is the server's so both readings are the same everywhere.
  messages: (id, run) => fetch(`/api/workers/${enc(id)}/messages${runQ(run)}`).then((r) => r.json()),
  frames: (id, run) => fetch(`/api/workers/${enc(id)}/frames${runQ(run)}`).then((r) => r.json()),
};

const enc = encodeURIComponent;
const runQ = (run) => (run ? `?run=${enc(run)}` : "");

/** How often the roster re-reads. Both endpoints are in-memory reads on the server — the
 *  live roster is a lock on a HashMap, the ended list a capped Vec — so this is cheap
 *  enough to be genuinely live rather than nearly-live. Held off while the page is hidden,
 *  since nothing is being read then. */
const POLL_MS = 2000;

// ── words ─────────────────────────────────────────────────────────────────────
// English is the default and the fallback.
//
// The title is **Sessions**, not "Workers". A worker is one rung — the bottom one — and
// this page has not been only about that rung for a while: the roster it lists is every
// live session on the ladder (Reaction, Deliberation, Cognition, Reflection) with the
// workers each one spawned nested under it, plus the sessions that just ended, plus any
// one of their wire logs. Titling all of that "Workers" named the leaves and dropped the
// tree. The id stays `workers` — that is the endpoint's name (`/api/workers`) and the
// route's, and those are addresses, not headings.
//
// So unlike the old title, this one is a plain word rather than this system's own
// vocabulary, and it is said in the reader's language.
//
// **The rung names are not in these tables at all** — see `ROLE` below. They are the one
// thing on this page that is this system's own vocabulary, so they are the same word in
// both languages and there is nothing to translate.
//
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped
// here. Until that exists, an unsupported language lands on English.
const T = {
  en: {
    title: "Sessions",
    runningN: (n) => `${n} running`,
    endedN: (n) => `${n} ended`,
    emptyBig: "Nothing is running right now.",
    emptySub: "That's the whole answer — not a page still loading.",
    noTitle: "(nothing said about what it's for)",
    // One word for what a session is doing with itself — the server folds `busy`+`queued`
    // into it, so there is nothing here to recombine and nothing to get wrong.
    state: { running: "running", waiting: "mail waiting", idle: "idle" },
    // How the last turn ended, drawn only when that is something to chase. `completed` has
    // no entry on purpose: a row that says so on every quiet card teaches the eye to skip
    // the line, and the line only exists for the cards where it is bad news.
    lastTurn: { failed: "last turn failed", interrupted: "last turn was stopped" },
    turns: (n) => (n === 1 ? "1 turn" : `${n} turns`),
    up: (t) => `up ${t}`,
    live: "Live", endedHead: "Just ended",
    // An owner that shut down while its worker kept running. The condition behind the
    // dropped report, so it is named rather than shown as a row with no parent.
    orphans: "Still running, owner gone",
    orphanNote: (id) => `owner ${id} is no longer registered`,
    // Which ledger task this session serves, and — the case worth seeing — that it serves
    // none. An unlinked worker is missing from its task's own line, so that task reads as
    // having nobody on it while this session is working it.
    onTask: (subject) => `on ${subject}`,
    unlinked: "not linked to any task",
    // On an ended row the tree is gone, so the owner is named by id — the only address
    // there is, and the one a reader can match against another row on the page.
    ownedBy: (id) => `owned by ${id}`,
    endedAgo: (t) => `ended ${t} ago`,
    // A restart row, said the same way and in the same breath as an ordinary end. It is not
    // a failure to flag: a rung's thread is handed straight back at boot, and a worker's is
    // offered to Cognition to resume — see `registry::index::{resumable, lost_workers}`. All
    // this word adds over "ended" is that nothing recorded the stop, which is why the time
    // it carries is the start.
    cutOffAgo: (t) => `cut off ${t} ago`,
    ranFor: (t) => `ran ${t}`,
    noEnded: "No sessions have ended yet.",
    // The session panel: the fold first, the record behind it.
    transcript: "What happened",
    frames: "Wire log",
    messagesOf: (n) => `${n} messages`,
    messagesTruncated: (shown, total) => `last ${shown} of ${total}`,
    noMessages: "Nothing in this session's log reads as a message.",
    noMessagesButFrames: (n) =>
      `${n} frames are on disk — this build could not fold them. The wire log has them verbatim.`,
    framesOf: (n) => `${n} frames`,
    framesTruncated: (shown, total) => `last ${shown} of ${total}`,
    noFrames: "No frames were kept for this session.",
    noFramesNote: "Nothing crossed the wire under this id, or its run's files are gone.",
    framesFailed: "That session's wire log could not be read.",
    // What each folded message was. Short: they sit in a fixed column beside the line.
    //
    // `agent` keeps the wire's own flat word, because every verb tried here was read as
    // a claim about delivery. An `agentMessage` is the model's own working-out and
    // reaches nobody; the only thing a person ever heard is a `hi_say` call, which is the
    // row below. "said" claimed the person had been answered when they got nothing;
    // "typed" answered that but made the reader stop and work out what it meant. `agent`
    // says whose line it is and promises nothing about where it went.
    kind: {
      user: "prompt",
      agent: "agent",
      say: "said",
      thinking: "thought",
      command: "ran",
      edit: "edited",
      tool: "tool",
      search: "searched",
      todo: "todo",
      compaction: "compacted",
      warning: "warning",
      stderr: "stderr",
      item: "item",
    },
    turnN: (n) => `Turn ${n}`,
    turnOpen: "never finished",
    tokens: (n) => `${n.toLocaleString()} tokens`,
    exit: (n) => (n === 0 ? "ok" : `exit ${n}`),
    tookMs: (ms) => (ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`),
    args: "arguments",
    result: "result",
    errored: "error",
    loading: "Reading…",
    close: "Close",
    send: "sent", recv: "received", stderr: "stderr",
    runLabel: (r) => `run ${r}`,
    sessionLabel: (id) => `session ${id}`,
    secs: (n) => `${n}s`, mins: (n) => `${n}m`, hours: (n) => `${n}h`, days: (n) => `${n}d`,
  },
  zh: {
    title: "会话",
    runningN: (n) => `${n} 个在跑`,
    endedN: (n) => `${n} 个结束了`,
    emptyBig: "现在没有活在跑。",
    emptySub: "这就是全部 —— 不是还没加载出来。",
    noTitle: "（没写这是干什么的）",
    state: { running: "在跑", waiting: "有没读的", idle: "闲着" },
    lastTurn: { failed: "上一轮挂了", interrupted: "上一轮被停了" },
    turns: (n) => `${n} 轮`,
    up: (t) => `已运行 ${t}`,
    live: "在跑", endedHead: "刚结束",
    orphans: "还在跑,派它的已经没了",
    orphanNote: (id) => `派它的 ${id} 已经不在了`,
    onTask: (subject) => `做 ${subject}`,
    unlinked: "没挂到任何任务",
    ownedBy: (id) => `归 ${id}`,
    endedAgo: (t) => `${t}前结束`,
    cutOffAgo: (t) => `${t}前被打断`,
    ranFor: (t) => `跑了 ${t}`,
    noEnded: "还没有结束的会话。",
    transcript: "做了什么",
    frames: "原始帧",
    messagesOf: (n) => `${n} 条`,
    messagesTruncated: (shown, total) => `共 ${total} 条,这是最后 ${shown} 条`,
    noMessages: "这个会话的日志里没有能读成消息的东西。",
    noMessagesButFrames: (n) => `盘上有 ${n} 帧,这个版本折不出来。原始帧那边是全的。`,
    framesOf: (n) => `${n} 帧`,
    framesTruncated: (shown, total) => `共 ${total} 帧,这是最后 ${shown} 帧`,
    noFrames: "这个会话没留下帧。",
    noFramesNote: "这个 id 下没有东西走过线,或者那次运行的文件已经没了。",
    framesFailed: "读不到这个会话的原始帧。",
    kind: {
      user: "收到",
      agent: "agent",
      say: "说",
      thinking: "想",
      command: "跑",
      edit: "改文件",
      tool: "工具",
      search: "搜",
      todo: "待办",
      compaction: "压缩上下文",
      warning: "警告",
      stderr: "标准错误",
      item: "条目",
    },
    turnN: (n) => `第 ${n} 轮`,
    turnOpen: "没跑完",
    tokens: (n) => `${n.toLocaleString()} tokens`,
    exit: (n) => (n === 0 ? "成功" : `退出码 ${n}`),
    tookMs: (ms) => (ms < 1000 ? `${ms} 毫秒` : `${(ms / 1000).toFixed(1)} 秒`),
    args: "参数",
    result: "结果",
    errored: "报错",
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

/** The ladder, top to bottom — the order `docs/arch/agents.md` gives: the mouth, the
 *  outward brain, the housekeeper. A role not named here sorts after these rather than
 *  being dropped, so a fourth rung appears instead of vanishing.
 *
 *  `deliberation` used to sit second and is gone from both tables. It names nothing:
 *  `identity::Role` has no such variant, so `role` can never carry the word, and
 *  `agents.md` records the rung as retired into Cognition. It survived here as a row in a
 *  lookup table, which is the quiet form of the thing this repo keeps paying for — a name
 *  for a mechanism that does not exist. */
const LADDER = ["reaction", "cognition", "reflection"];

/** What each rung and each kind of worker is **called** — `docs/arch/agents.md`, the same
 *  words as the `role` field this page reads and the `X-HI-Role` header the sessions
 *  themselves carry.
 *
 *  These used to be descriptions of what each rung *does*: `speaking`, `mulling`,
 *  `thinking`, `filing`. Two things were wrong with that, and both showed up on screen at
 *  once. A present participle reads as a live status, and the actual status sits two lines
 *  below it — so a row said `speaking` and then `idle`, which is a contradiction if you
 *  read the pill as it is written. And the task line beside it already names the rung, and
 *  names it better: `speaking · what reaches the person`, `thinking · the shared brain`. The pill was
 *  saying the same thing twice, in the worse of the two words.
 *
 *  Not translated, in either direction. A rung's name is this system's own vocabulary
 *  (`views/factory.rs`, rule 3) — it names a part of this architecture, not an ordinary
 *  object — so there is one table here rather than one per language. */
const ROLE = {
  reaction: "Reaction",
  cognition: "Cognition",
  reflection: "Reflection",
  worker: "Worker",
};

/** A worker's specialism, when it has one. A general worker is just a `Worker`, which the
 *  role above already says.
 *
 *  Every type in `identity::WorkerType` except `general` belongs here. A type missing from
 *  this table does not fail — it falls through to the bare `Worker` the role gives, which is
 *  indistinguishable from a general session on screen. That is how every `person-reader` a
 *  settling pass dispatched read as an unnamed worker: the variant was added to the enum and
 *  never here. */
const TYPE = {
  "view-builder": "View builder",
  "view-reviewer": "View reviewer",
  "decision-maker": "Decision maker",
  "drive-organizer": "Drive organizer",
  "person-reader": "Person reader",
  "task-manager": "Task manager",
};

function label(row) {
  return TYPE[row.type] || ROLE[row.role] || row.role;
}

/* Which ledger task a session serves, as a meta chip — and, for a worker serving none, that
 *  it serves none.
 *
 *  Only workers are asked. The three rungs are standing and belong to no task; marking them
 *  "not linked" would put the warning on every page, on rows where it is not a fault, which
 *  is how a warning stops being read.
 *
 *  The unlinked case is the one this exists for. A worker with no subject is absent from its
 *  task's own line in the ledger, so that task reads as having nobody on it while this
 *  session is working it — and the natural response to that is to start a second worker.
 *  Here it is the only place both facts are on one screen.
 *
 *  And it is asked only of the kinds that serve the ledger at all. A `person-reader` is one
 *  of Reflection's organizers — one per person present in a stretch, keyed to a
 *  `people/<name>` facet, never a task anyone is owed — so it has no subject to be missing.
 *  Marking every one of them is the same mistake as marking the rungs, at fan-out scale: the
 *  settling pass starts a reader per person, and a page of red warnings on rows where none is
 *  a fault is a page with no warnings on it. The rule lives in
 *  `WorkerType::expects_a_subject`; this is its half. */
const NO_SUBJECT_EXPECTED = new Set(["person-reader"]);

function taskLink(row) {
  if (row.role !== "worker") return null;
  if (row.subject) return { text: L.onTask(row.subject), warn: false };
  if (NO_SUBJECT_EXPECTED.has(row.type)) return null;
  return { text: L.unlinked, warn: true };
}

export default function Workers() {
  const [live, setLive] = useState(null);
  const [ended, setEnded] = useState([]);
  // The open card is held by its **address** — `{ id, run }` — not by the row object,
  // because the poll below replaces every row on each tick and a held object would freeze
  // the panel on the version that was clicked. A live card passes no run; the endpoint
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

  const roots = useMemo(() => forest(live || []), [live]);

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

        <h2 className="hi-workers__section-head">{L.live}</h2>
        {running === 0 ? (
          <div className="hi-workers__empty">
            <div className="hi-workers__empty-big">{L.emptyBig}</div>
            <div className="hi-workers__empty-sub">{L.emptySub}</div>
          </div>
        ) : (
          <Canvas roots={roots} open={open} setOpen={setOpen} />
        )}

        {/* Under the tree, never beside it. A tree is as wide as the delegation happens to
            be, and a column next to it would be squeezed by the one thing this page is for.
            Full width, two cards deep, and it scrolls — the list is capped at 40 and every
            one of them is the same size, so "two deep" is a height and not an estimate. */}
        <h2 className="hi-workers__section-head hi-workers__section-head--ended">{L.endedHead}</h2>
        {ended.length === 0 ? (
          <div className="hi-workers__none">{L.noEnded}</div>
        ) : (
          <div className="hi-workers__band">
            <div className="hi-workers__band-grid">
              {ended.map((e) => (
                <EndedCard key={`${e.run}:${e.session}`} row={e} open={open} setOpen={setOpen} />
              ))}
            </div>
          </div>
        )}
      </div>

      {open && <Session addr={open} onClose={() => setOpen(null)} />}
    </div>
  );
}

/* ── the tree ─────────────────────────────────────────────────────────────────
   Three steps, kept apart on purpose: `forest` turns the flat roster into ownership,
   `layout` turns ownership into geometry, and `Canvas` draws geometry. Only the middle
   one knows what a pixel is, and only the last one knows what a DOM node is. */

/** The ownership forest, from the flat roster and nothing else.
 *
 *  Keyed on the owner **id**, which is why the endpoint reports one. It used to report the
 *  owner's role *word* while the owner was live and its bare id only once the owner had
 *  died — so the id, the one thing a tree can be built from, was there exactly when it was
 *  useless. That the word worked at all was a coincidence of one session per rung being
 *  live at a time; it is a label, not an address.
 *
 *  Depth is whatever the data says, which today is two ranks and no more: `hi_create_worker`
 *  is refused at dispatch to any role but `cognition` and `reflection` (`mcp::dispatch_tool`),
 *  so a rung is the only thing that owns anything, and a worker's own fan-out is sub-agents
 *  living inside its session that the switchboard never sees. The recursion is here anyway
 *  because the shape belongs to `owner` and not to this file: the day a worker may dispatch,
 *  the third rank draws itself rather than being flattened into the second — which is what
 *  the old one-indent nesting would have done, silently, as a worker reading as a sibling of
 *  its own owner.
 *
 *  Two shapes are not trees and both are drawn rather than dropped, because a live session
 *  missing from this page is the one failure it cannot have:
 *
 *  - **An orphan** — a session naming an owner the roster does not have, i.e. the owner shut
 *    down while it kept running. That is the dropped-report condition (`gaps.md` §3), so it
 *    becomes a root that says so rather than a row silently reparented to the top.
 *  - **A cycle** — impossible by construction and therefore worth surviving anyway. Members
 *    of one are parented but reachable from no root, so a sweep promotes the first of them
 *    to a root and cuts the edge that would otherwise draw it twice.
 */
function forest(rows) {
  const nodes = new Map(rows.map((r) => [r.id, { key: r.id, row: r, children: [], orphan: false }]));
  const parent = new Map();
  const roots = [];

  for (const r of rows) {
    const n = nodes.get(r.id);
    const owner = r.owner && r.owner !== r.id ? nodes.get(r.owner) : null;
    if (owner) {
      owner.children.push(n);
      parent.set(n.key, owner);
    } else {
      n.orphan = !!r.owner;
      roots.push(n);
    }
  }

  const seen = new Set();
  const mark = (n) => {
    if (seen.has(n.key)) return;
    seen.add(n.key);
    n.children.forEach(mark);
  };
  roots.forEach(mark);
  for (const n of nodes.values()) {
    if (seen.has(n.key)) continue;
    const p = parent.get(n.key);
    if (p) p.children = p.children.filter((c) => c !== n);
    n.orphan = true;
    roots.push(n);
    mark(n);
  }

  const rank = (n) => {
    const i = LADDER.indexOf(n.row.role);
    return i === -1 ? LADDER.length : i;
  };
  const byStart = (a, b) => String(a.row.started).localeCompare(String(b.row.started));
  // Ladder first, then oldest first inside a rank — a session's place must not jump around
  // between two-second polls.
  roots.sort((a, b) => rank(a) - rank(b) || byStart(a, b));
  const deep = (n) => { n.children.sort(byStart); n.children.forEach(deep); };
  roots.forEach(deep);
  return roots;
}

/** Every card in one drawing is the same width — a tidy layout needs one number for a
 *  node's width, and a session has no natural one. `NODE_W` is what a card wants: enough
 *  for the field that needs the most room, a `doing` line of monospace. `MIN_NODE_W` is
 *  the least it will take to keep the tree inside the frame, below which the canvas
 *  scrolls instead. Narrowing is the better of the two, up to a point: a rank that runs
 *  off the right edge hides sessions, and a card 50px narrower only wraps a line. */
const NODE_W = 236;
const MIN_NODE_W = 170;
/** Between two sibling subtrees, between two whole trees, and between depth rows. The row
 *  gap is the arrow's room: shorter and the arrowhead lands on the card above it. */
const SIB_GAP = 16;
const TREE_GAP = 44;
const ROW_GAP = 46;
/** What a card is assumed to be tall before it has been measured — see `useHeights`. Only
 *  the first frame after a card appears uses it, and being wrong only means that card's
 *  row settles into place instead of arriving there. */
const EST_H = 124;
/** The arrowhead's length, taken off the end of the curve so the two do not overlap. */
const HEAD = 9;

/** Ownership → geometry. The whole engine; it touches no DOM and returns only numbers.
 *
 *  A tidy tree in the shape of Reingold–Tilford, adapted the one way this data needs:
 *  node heights are *measured*, not assumed, because a card carrying a wrapped `doing`
 *  line is half again as tall as one carrying nothing.
 *
 *  - **Widths, bottom-up.** A subtree's band is the wider of one card and the row its
 *    children need. That single rule is what stops two subtrees overlapping no matter how
 *    lopsided the delegation gets.
 *  - **Placement, top-down.** Children are laid across their parent's band, then the parent
 *    is centred over the *centres of its first and last child* rather than over the band —
 *    an outsized first child otherwise drags its owner off the children it owns. The result
 *    is clamped back inside the band, since a parent leaning out of its own band is a
 *    parent overlapping its sibling.
 *  - **One row per depth**, at the tallest measured card in it, so the tree reads in ranks:
 *    what the system started on top, what those spawned under them, leaves at the bottom.
 */
function layout(roots, heights, nodeW) {
  const heightOf = (n) => heights[n.key] || EST_H;

  const rowH = [];
  const rank = (n, d) => {
    n.depth = d;
    rowH[d] = Math.max(rowH[d] || 0, heightOf(n));
    for (const c of n.children) rank(c, d + 1);
  };
  for (const r of roots) rank(r, 0);

  const rowY = [];
  let y = 0;
  for (let d = 0; d < rowH.length; d++) {
    rowY[d] = y;
    y += rowH[d] + ROW_GAP;
  }
  const height = Math.max(0, y - ROW_GAP);

  const span = (n) =>
    n.children.reduce((s, c) => s + c.band, 0) + SIB_GAP * Math.max(0, n.children.length - 1);
  const band = (n) => {
    for (const c of n.children) band(c);
    n.band = Math.max(nodeW, span(n));
  };
  for (const r of roots) band(r);

  const nodes = [];
  const edges = [];
  const place = (n, left) => {
    let cx;
    if (n.children.length) {
      let x = left + (n.band - span(n)) / 2;
      for (const c of n.children) {
        place(c, x);
        x += c.band + SIB_GAP;
      }
      const kids = n.children;
      cx = (kids[0].cx + kids[kids.length - 1].cx) / 2;
      const lo = left + nodeW / 2;
      const hi = left + n.band - nodeW / 2;
      cx = Math.min(Math.max(cx, lo), hi);
    } else {
      cx = left + n.band / 2;
    }
    n.cx = Math.round(cx);
    n.x = Math.round(cx - nodeW / 2);
    n.y = rowY[n.depth];
    n.h = heightOf(n);
    n.w = nodeW;
    nodes.push(n);
    for (const c of n.children) edges.push({ key: `${n.key}>${c.key}`, from: n.key, to: c.key });
  };

  let x = 0;
  for (const r of roots) {
    place(r, x);
    x += r.band + TREE_GAP;
  }

  return { nodes, edges, nodeW, width: Math.max(0, x - TREE_GAP), height };
}

/** The engine, run at the widest card that keeps the drawing inside `avail`.
 *
 *  Total width is monotone in card width and very nearly proportional to it, so two or
 *  three passes land on the answer; the loop is bounded rather than solved because the
 *  relation is only *nearly* proportional — a subtree whose band is set by its own card
 *  rather than by its children does not shrink with the rest.
 *
 *  It depends on the frame's width and not at all on the measured heights, so it cannot
 *  oscillate with them: narrowing the cards makes them taller, and taller changes only how
 *  far apart the ranks sit. */
function fitted(roots, heights, avail) {
  let w = NODE_W;
  let plan = layout(roots, heights, w);
  for (let i = 0; i < 5 && avail > 0 && plan.width > avail && w > MIN_NODE_W; i++) {
    w = Math.max(MIN_NODE_W, Math.floor(w * (avail / plan.width)));
    plan = layout(roots, heights, w);
  }
  return plan;
}

/** The arrow from an owner down to one session it created — drawn from where the two cards
 *  *are*, which during a move is not where the engine put them. Leaves and re-enters
 *  vertically, so the head always points straight down and needs no rotation. */
function wireOf(from, to) {
  const x1 = from.x + from.w / 2;
  const y1 = from.y + from.h;
  const x2 = to.x + to.w / 2;
  const y2 = to.y - HEAD;
  const d = Math.max(14, (y2 - y1) * 0.5);
  return `M ${x1} ${y1} C ${x1} ${y1 + d}, ${x2} ${y2 - d}, ${x2} ${y2}`;
}

function headOf(to) {
  const x = to.x + to.w / 2;
  const y = to.y;
  return `M ${x - 5} ${y - HEAD} L ${x + 5} ${y - HEAD} L ${x} ${y} Z`;
}

/** Measure every card, because the engine lays rows out at the tallest one in them and a
 *  card's height is its content's. A `ResizeObserver` rather than a pass after each render:
 *  a card also changes height without any data changing — a web font arriving, the frame
 *  narrowing under a `doing` line that then wraps — and a layout that only re-measured on
 *  poll would hold the stale row height until the next tick.
 *
 *  Border-box, so the transform the animator is writing onto the same element (a scale, on
 *  the way in) cannot be read back as a height and fed to the engine. */
function useHeights() {
  const [heights, setHeights] = useState({});
  const seen = useRef({});
  const refs = useRef(new Map());
  const ro = useRef(null);

  if (ro.current === null && typeof ResizeObserver !== "undefined") {
    ro.current = new ResizeObserver((entries) => {
      let dirty = false;
      for (const e of entries) {
        const key = e.target.getAttribute("data-key");
        if (!key) continue;
        const h = Math.round(e.target.offsetHeight);
        if (h > 0 && seen.current[key] !== h) {
          seen.current[key] = h;
          dirty = true;
        }
      }
      if (dirty) setHeights({ ...seen.current });
    });
  }

  useEffect(() => () => ro.current && ro.current.disconnect(), []);

  /** One stable callback per key, cached — a fresh function each render would have React
   *  detach and re-attach every card's ref on every poll. */
  const measure = useCallback((key) => {
    let fn = refs.current.get(key);
    if (!fn) {
      fn = (el) => {
        if (fn.el && fn.el !== el && ro.current) ro.current.unobserve(fn.el);
        fn.el = el || null;
        if (el) {
          if (ro.current) ro.current.observe(el);
        } else {
          refs.current.delete(key);
          delete seen.current[key];
        }
      };
      refs.current.set(key, fn);
    }
    return fn;
  }, []);

  return [heights, measure];
}

/** How much room the drawing has, which is the frame's width and not the window's. The two
 *  are different by whatever the host has inset this view to, and by a scrollbar; measuring
 *  the element is the only reading that is true of both. Zero until the first measurement,
 *  which `fitted` reads as "no constraint yet" and draws at the width a card wants.
 *
 *  Hands back the element too, because the other question about this box — which way it has
 *  tree behind it — can only be answered by reading its live scroll position. */
function useFrame() {
  const [avail, setAvail] = useState(0);
  const ro = useRef(null);
  const el = useRef(null);

  const frame = useCallback((node) => {
    if (ro.current) { ro.current.disconnect(); ro.current = null; }
    el.current = node || null;
    if (!node || typeof ResizeObserver === "undefined") return;
    setAvail(Math.round(node.clientWidth));
    ro.current = new ResizeObserver(() => {
      if (el.current) setAvail(Math.round(el.current.clientWidth));
    });
    ro.current.observe(node);
  }, []);

  useEffect(() => () => ro.current && ro.current.disconnect(), []);
  return [avail, frame, el];
}

/** How fast a card slides to a new place, and how fast one fades in or out, as the
 *  time constant of an exponential ease. Position is slower than opacity on purpose: a
 *  session appearing should be *seen* appearing, and the tree it displaces should have
 *  finished moving by the time the eye gets there. */
const EASE_MOVE = 110;
const EASE_FADE = 70;

/** Whether the person has asked for less of this. Read once per frame rather than once per
 *  mount, because the setting can change under a page that is already open — and the
 *  animator is the only thing on this canvas that moves a card, so honouring it anywhere
 *  else would honour it nowhere. */
function stillness() {
  return typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** The one animator, owning card positions and arrow geometry together.
 *
 *  It exists because they cannot be owned separately. A card is placed at the engine's
 *  coordinates and an arrow is a path between two of them; hand the card's motion to CSS
 *  and the arrow either snaps to the target while the card is still travelling — visibly
 *  detached from the thing it points at — or has to be re-rendered every frame by React to
 *  keep up. So one rAF pass eases both and writes both straight to the DOM: `transform` and
 *  `opacity` on each card, `d` on each arrow, recomputed from where the cards *currently
 *  are*. React is not re-rendered by any of it.
 *
 *  It also owns the leaving. A session that ends is gone from the next poll's roster, and
 *  unmounting it there would make the one moment worth watching the one moment with no
 *  animation at all. So the scene outlives the roster: a card whose session is gone keeps
 *  its place and fades, and only when it is invisible does the scene drop it and ask React
 *  for the one re-render that unmounts it.
 *
 *  The loop stops when nothing is moving — which is nearly always, since the roster only
 *  changes when a session starts or ends. */
function useCanvas(plan) {
  const scene = useRef({ nodes: new Map(), edges: new Map(), synced: null });
  const els = useRef({ nodes: new Map(), wires: new Map(), heads: new Map() });
  const binders = useRef({ nodes: new Map(), wires: new Map(), heads: new Map() });
  const raf = useRef(0);
  const [, bump] = useReducer((n) => n + 1, 0);

  // Fold the new geometry into the living scene. Done during render rather than in an
  // effect so a session that has just appeared is drawn on this frame and not the next;
  // idempotent per plan, so a double render costs nothing.
  const s = scene.current;
  if (s.synced !== plan) {
    s.synced = plan;
    const live = new Set();
    for (const n of plan.nodes) {
      live.add(n.key);
      const cur = s.nodes.get(n.key);
      if (cur) {
        cur.row = n.row;
        cur.tx = n.x;
        cur.ty = n.y;
        cur.h = n.h;
        cur.w = n.w;
        cur.gone = false;
      } else {
        // A new card starts *at* its place and fades up. Sliding it in from somewhere
        // would be a claim about where it came from, and nothing here knows that.
        s.nodes.set(n.key, { key: n.key, row: n.row, x: n.x, y: n.y, tx: n.x, ty: n.y, h: n.h, w: n.w, a: 0, gone: false });
      }
    }
    for (const n of s.nodes.values()) if (!live.has(n.key)) n.gone = true;

    const liveEdges = new Set();
    for (const e of plan.edges) {
      liveEdges.add(e.key);
      const cur = s.edges.get(e.key);
      if (cur) cur.gone = false;
      else s.edges.set(e.key, { ...e, a: 0, gone: false });
    }
    for (const e of s.edges.values()) if (!liveEdges.has(e.key)) e.gone = true;
  }

  // Restart the loop after every render: a render is the only thing that can have given
  // the scene something new to settle.
  useEffect(() => {
    if (raf.current) return undefined;
    let last = 0;
    const step = (t) => {
      raf.current = 0;
      const dt = last ? Math.min(64, t - last) : 16;
      last = t;
      const calm = stillness();
      const km = calm ? 1 : 1 - Math.exp(-dt / EASE_MOVE);
      const kf = calm ? 1 : 1 - Math.exp(-dt / EASE_FADE);
      const sc = scene.current;
      const dead = [];
      let busy = false;
      let membership = false;

      for (const n of sc.nodes.values()) {
        const ta = n.gone ? 0 : 1;
        n.x += (n.tx - n.x) * km;
        n.y += (n.ty - n.y) * km;
        n.a += (ta - n.a) * kf;
        if (Math.abs(n.tx - n.x) < 0.4) n.x = n.tx; else busy = true;
        if (Math.abs(n.ty - n.y) < 0.4) n.y = n.ty; else busy = true;
        if (Math.abs(ta - n.a) < 0.01) n.a = ta; else busy = true;
        const el = els.current.nodes.get(n.key);
        if (el) {
          el.style.transform = `translate3d(${n.x}px, ${n.y}px, 0) scale(${(0.94 + 0.06 * n.a).toFixed(3)})`;
          el.style.opacity = n.a;
        }
        if (n.gone && n.a === 0) dead.push(n.key);
      }

      for (const e of sc.edges.values()) {
        const from = sc.nodes.get(e.from);
        const to = sc.nodes.get(e.to);
        // An arrow is never more present than the fainter of the two cards it joins, so
        // it cannot outlive either of them or arrive before both.
        const ceiling = from && to ? Math.min(from.a, to.a) : 0;
        const ta = Math.min(e.gone ? 0 : 1, ceiling);
        e.a += (ta - e.a) * kf;
        if (Math.abs(ta - e.a) < 0.01) e.a = ta; else busy = true;
        const wire = els.current.wires.get(e.key);
        const head = els.current.heads.get(e.key);
        if (from && to && wire) wire.setAttribute("d", wireOf(from, to));
        if (from && to && head) head.setAttribute("d", headOf(to));
        if (wire) wire.style.opacity = e.a;
        if (head) head.style.opacity = e.a;
        if ((e.gone || !from || !to) && e.a === 0) {
          sc.edges.delete(e.key);
          membership = true;
        }
      }

      // After the arrows, never before: an arrow is drawn from the two cards it joins, so
      // a card cannot leave the scene in the frame its own arrow is still being placed.
      for (const key of dead) {
        sc.nodes.delete(key);
        membership = true;
      }

      if (busy) raf.current = requestAnimationFrame(step);
      if (membership) bump();
    };
    raf.current = requestAnimationFrame(step);
    return undefined;
  });

  useEffect(() => () => { if (raf.current) cancelAnimationFrame(raf.current); }, []);

  /** One stable ref callback per key per layer, for the same reason `useHeights` caches
   *  its own: a fresh function each render detaches and re-attaches every element. */
  const bind = useCallback((layer, key) => {
    const store = els.current[layer];
    const cache = binders.current[layer];
    let fn = cache.get(key);
    if (!fn) {
      fn = (el) => {
        if (el) store.set(key, el);
        else { store.delete(key); cache.delete(key); }
      };
      cache.set(key, fn);
    }
    return fn;
  }, []);

  return [scene.current, bind];
}

/** The tree on screen: arrows underneath, cards on top, both in the engine's coordinates.
 *
 *  It scrolls sideways rather than folding, and that is the honest failure mode: a tree
 *  that wrapped would be a different tree. When it fits, it centres. */
function Canvas({ roots, open, setOpen }) {
  const [heights, measure] = useHeights();
  const [avail, frame, box] = useFrame();
  const plan = useMemo(() => fitted(roots, heights, avail), [roots, heights, avail]);
  const [scene, bind] = useCanvas(plan);

  // Which way there is more tree than frame. A scroller that has run out of narrowing is
  // hiding sessions, and on this page a session you cannot see is the failure the whole
  // surface exists to prevent — so the edge it is hiding them past says so. macOS overlay
  // scrollbars are invisible until you are already scrolling, which is too late to be the
  // thing that tells you there is more.
  //
  // Read off the element both times, never inferred from the plan. The roster is replaced
  // every poll, so a fade derived from "is the plan wider than the frame" would be
  // recomputed twice a minute with no idea where the reader had scrolled to — and would put
  // the right-hand fade back over a tree that is already scrolled to its end.
  const [more, setMore] = useState("");
  const sense = useCallback(() => {
    const el = box.current;
    if (!el) return;
    const left = el.scrollLeft > 1;
    const right = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
    setMore(left && right ? "both" : left ? "left" : right ? "right" : "");
  }, [box]);
  // And again whenever the drawing changes shape under it: a session ending can make a tree
  // that overflowed fit, and nothing scrolled.
  useEffect(sense, [sense, plan, avail]);

  const nodes = [...scene.nodes.values()];
  const edges = [...scene.edges.values()];
  const lit = (key) => !!open && !open.run && open.id === key;

  return (
    <div className="hi-workers__reel" data-more={more || undefined}>
      <div className="hi-workers__canvas" ref={frame} onScroll={sense}>
        <div className="hi-workers__plot" style={{ width: plan.width, height: plan.height }}>
          <svg
            className="hi-workers__wires"
            width={plan.width}
            height={plan.height}
            viewBox={`0 0 ${plan.width} ${plan.height}`}
            aria-hidden="true"
          >
            {edges.map((e) => (
              <g key={e.key} data-lit={lit(e.from) || lit(e.to) ? "true" : undefined}>
                <path className="hi-workers__wire" ref={bind("wires", e.key)} d="" />
                <path className="hi-workers__tip" ref={bind("heads", e.key)} d="" />
              </g>
            ))}
          </svg>

          {nodes.map((n) => (
            <div
              key={n.key}
              ref={bind("nodes", n.key)}
              className="hi-workers__node"
              style={{ transform: `translate3d(${n.x}px, ${n.y}px, 0)`, opacity: n.a, width: n.w }}
              data-gone={n.gone ? "true" : undefined}
            >
              <LiveCard nodeKey={n.key} row={n.row} measure={measure} open={open} setOpen={setOpen} />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

/** One live session.
 *
 *  **A card has one state, and which fields are meaningful is a function of it.** That rule
 *  is the whole of this component, and it is here because the page has now produced the
 *  same contradiction three times: the role pills once said `speaking`/`thinking` beside a
 *  status of `idle` (fixed by renaming them to the rung); `doing` said `thinking` beside
 *  `idle` one line down; and `queued` was drawn as a chip on a third axis, so a session
 *  with work already in its inbox read as plain `idle` with an easily-missed word beside
 *  it. Each was patched as its own bug. They are one bug: independent fields rendered as
 *  though they were independent facts.
 *
 *  So the server now sends one `state` — `running` · `waiting` · `idle` — and the fields
 *  below are gated on it rather than drawn unconditionally.
 *
 *  `doing` is the case that matters. It is *only* meaningful while a turn is in flight:
 *  nothing clears it when one ends, and there is nothing it could honestly be cleared to,
 *  because it is the last thing seen and that stays true. An idle session finished its
 *  turn — there is no "what it is doing" — and what a reader wants from it is `tail`, what
 *  it *said*, which is on the line below and was always there.
 */
function LiveCard({ nodeKey, row, measure, open, setOpen }) {
  // A live card is addressed by id with no run, so it cannot collide with an ended one that
  // happens to share the number — which, across runs, they routinely do.
  const isOpen = !!open && open.id === row.id && !open.run;
  // An unknown state word from a newer server reads as idle rather than blanking the card:
  // the roster showing a session at all is the load-bearing part.
  const state = L.state[row.state] ? row.state : "idle";
  const running = state === "running";
  // **The card's one blind spot until now: `idle` was the word for both endings.** A
  // worker that answered its brief and one whose turn died on a 429 drew the same card with
  // the same clock, and the only surface that knew the difference was the message fold
  // inside the panel — which you have to open a session to reach. On 2026-08-18 that cost
  // three workers a recovery: they read as idle, and were told to stop idling.
  //
  // Drawn on a quiet card only, for the same reason `doing` is drawn on a busy one — it is
  // about a turn that is over, and beside `running` it describes the turn before the one in
  // flight. A clean `completed` draws nothing: the field is here for bad news.
  const ended =
    !running && row.last_turn && row.last_turn.outcome !== "completed" ? row.last_turn : null;
  // Named rather than shown as a card hanging off nothing — an orphan silently reparented
  // to the top of the tree is the dropped-report condition made invisible all over again.
  const orphan = !!row.owner && !row.owner_role;
  return (
    <button
      type="button"
      data-key={nodeKey}
      ref={measure(nodeKey)}
      className="hi-workers__card"
      data-open={isOpen ? "true" : undefined}
      data-state={state}
      data-orphan={orphan ? "true" : undefined}
      aria-expanded={isOpen}
      onClick={() => setOpen(isOpen ? null : { id: row.id, run: null })}
    >
      {orphan && <span className="hi-workers__orphan">{L.orphans}</span>}

      {/* The card's top strip: what this is, and how it is. The two facts that are true
          of the session as a whole, in a fixed place on every card, so a row of them
          is scannable without reading any of the prose below. */}
      <span className="hi-workers__top">
        <span className={`hi-workers__dot${running ? " is-busy" : ""}`} aria-hidden />
        <span className="hi-workers__pill">{label(row)}</span>
        {/* The answer to the question this page exists to ask, so it takes the slot the eye
            lands on. It reads `idle 4m`, not `5m`: `started` measures uptime, which is the
            same number for a session quiet since breakfast and one that finished a turn two
            seconds ago. How long it has been *like this* is the part that says whether
            anything is wrong. */}
        <span className="hi-workers__state">
          {L.state[state]} {elapsed(row.state_since || row.started)}
        </span>
      </span>

      {/* Its own line, and exactly one. The server sends a headline now rather than the
          brief a worker was sent (`Status::title`), so there is no paragraph left to fit and
          nothing gained by giving it two lines — a tree whose titles are all one line is the
          thing that actually scans. Anything over the cap arrives already cut with an
          ellipsis; the CSS clamp is the backstop for a wide character set. */}
      <span className="hi-workers__title">{row.title || L.noTitle}</span>

      {/* The durable facts, and nothing that could contradict the line above. */}
      <span className="hi-workers__meta">
        {typeof row.turns === "number" && <span>{L.turns(row.turns)}</span>}
        {row.started && <span>{L.up(elapsed(row.started))}</span>}
        {(() => {
          const link = taskLink(row);
          return link ? (
            <span className={link.warn ? "is-warn" : undefined}>{link.text}</span>
          ) : null;
        })()}
        {/* Only once the owner has gone. While it is live the arrow already shows who it
            is, and repeating it on every child is noise. */}
        {orphan && <span className="is-warn">{L.orphanNote(row.owner)}</span>}
      </span>

      {/* Doing and said are different questions and never share a line. A card with a
          `doing` and no `tail` is a session working in silence — which is exactly what used
          to be indistinguishable from a dead one.

          The age beside it is what separates working from hung: `$ cargo test` four minutes
          in is a build, the same line forty minutes in is a session nothing will come back
          from. No threshold is applied — the number is shown and the reader judges, because
          picking the minute at which this page silently declares something dead is not a
          call it is in a position to make. */}
      {running && row.doing && (
        <span className="hi-workers__doing">
          {row.doing}
          {row.doing_at && <span className="hi-workers__age"> · {elapsed(row.doing_at)}</span>}
        </span>
      )}

      {/* The quiet card's half of that same slot: what a session is doing answers "is it
          alive", and how its last turn ended answers "did it get anywhere" — one of the two
          questions is live at a time, so they share the position and never the line.

          The reason is carried whole rather than summarised: `429 Too Many Requests` and a
          crashed subprocess call for different moves, and a card that said only "failed"
          would send the reader into the panel every time. An unknown word from a newer
          server prints itself rather than blanking the line. */}
      {ended && (
        <span className="hi-workers__ended">
          {L.lastTurn[ended.outcome] || ended.outcome}
          {ended.error ? `: ${ended.error}` : ""}
          {ended.at && <span className="hi-workers__age"> · {elapsed(ended.at)}</span>}
        </span>
      )}
      {row.tail && <span className="hi-workers__tail">{row.tail}</span>}
    </button>
  );
}

/** One session in the band under the tree. Every card in there is the same height, which is
 *  what lets the band be exactly two of them deep — so this one is built to a budget rather
 *  than to its content: a strip, a title, and one line of the facts that fit. Whatever does
 *  not fit is a click away, same as everywhere else on this page. */
function EndedCard({ row, open, setOpen }) {
  const lost = row.how === "restart";
  const isOpen = !!open && open.id === String(row.session) && open.run === row.run;
  // A restart card has no end — nothing recorded one — so it is dated by its start.
  const when = row.ended || row.started;
  return (
    <button
      type="button"
      className="hi-workers__card is-ended"
      data-open={isOpen ? "true" : undefined}
      aria-expanded={isOpen}
      onClick={() => setOpen(isOpen ? null : { id: String(row.session), run: row.run })}
    >
      <span className="hi-workers__top">
        <span className="hi-workers__dot" aria-hidden />
        <span className="hi-workers__pill">{label(row)}</span>
        <span className="hi-workers__state">
          {when ? (lost ? L.cutOffAgo(elapsed(when)) : L.endedAgo(elapsed(when))) : ""}
        </span>
      </span>

      <span className="hi-workers__title">{row.title || L.noTitle}</span>

      {/* One clamped run of text rather than the live card's wrapping row of chips. A
          wrapping row is however many lines its longest item makes it — a card whose
          subject is `kt8-067-parallel-slice-reassembly-20260820` took three where its
          neighbour took one — and this card's height is fixed, so the difference came out
          as lettering sliced in half along the bottom edge. Joined and clamped, what does
          not fit ends in an ellipsis instead, which is the ordinary way to say there is
          more and is already how the title behaves one line up. */}
      <span className="hi-workers__meta is-run-on">
        {[
          typeof row.turns === "number" ? L.turns(row.turns) : null,
          row.started && row.ended ? L.ranFor(between(row.started, row.ended)) : null,
          row.subject ? L.onTask(row.subject) : null,
          row.owner ? L.ownedBy(row.owner) : null,
        ]
          .filter(Boolean)
          .join(" · ")}
      </span>
    </button>
  );
}

/** The zoom-in: one session, read two ways.
 *
 *  **Messages by default.** The frame log is the record and the record is not a reading of
 *  itself: one sentence the agent said arrives as an `item/started`, hundreds of deltas and
 *  an `item/completed`, so frame-per-row is a wall of fragments — 11,891 frames across the
 *  logs this was measured on, against 369 things that actually happened. The fold is the
 *  server's (`foundation::codex::messages`), not this file's, so `curl` sees the same
 *  reading the page does.
 *
 *  **And the verbatim log is still one click away**, because it is the thing the fold can be
 *  wrong about. Each message carries the frame span it was folded from (`seq`..`through`),
 *  so a row here and a row there name the same moment. */
function Session({ addr, onClose }) {
  // Which reading is on screen. Messages first: it answers "what happened", which is why
  // someone opened a session. The wire answers "what crossed", which is what you fall back
  // to when you do not believe the first answer.
  const [reading, setReading] = useState("messages");
  const [state, setState] = useState({ status: "loading" });
  const [openRow, setOpenRow] = useState(null);
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
    setOpenRow(null);
    const read = reading === "messages" ? api.messages : api.frames;
    read(addr.id, addr.run)
      // The payload says which reading it answers. Without that the render between the
      // click and this effect draws the *previous* reading's payload through the new
      // reading's shape — and the two disagree about the word `frames`, which is the array
      // in one and the count of it in the other. That is not a mismatch that renders oddly;
      // it is `frames.map is not a function` and a blank panel.
      .then((d) => alive && setState({ status: "ok", of: reading, ...d }))
      .catch(() => alive && setState({ status: "failed", of: reading }));
    return () => { alive = false; };
  }, [addr.id, addr.run, reading]);

  // Anything from the other reading is still in flight as far as this one is concerned.
  const shown = state.of === reading ? state : { status: "loading" };
  const frames = shown.frames || [];
  const messages = shown.messages || [];
  const onMessages = reading === "messages";
  const count = onMessages
    ? shown.status === "ok" && messages.length
      ? shown.truncated
        ? L.messagesTruncated(messages.length, shown.total)
        : L.messagesOf(shown.total)
      : null
    : shown.status === "ok" && frames.length
      ? shown.truncated
        ? L.framesTruncated(frames.length, shown.total)
        : L.framesOf(shown.total)
      : null;

  return (
    <div className="hi-workers__scrim" onClick={onClose}>
      <div
        ref={panel}
        className="hi-workers__panel"
        role="dialog"
        aria-modal="true"
        aria-label={onMessages ? L.transcript : L.frames}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="hi-workers__panel-head">
          <div className="hi-workers__panel-title">
            <div className="hi-workers__readings" role="tablist">
              {[["messages", L.transcript], ["frames", L.frames]].map(([key, label]) => (
                <button
                  key={key}
                  type="button"
                  role="tab"
                  aria-selected={reading === key}
                  className="hi-workers__reading"
                  data-on={reading === key ? "true" : undefined}
                  onClick={() => setReading(key)}
                >
                  {label}
                </button>
              ))}
            </div>
            <span className="hi-workers__panel-sub">
              {[
                L.sessionLabel(addr.id),
                shown.run ? L.runLabel(shown.run) : null,
                count,
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
          {shown.status === "loading" && <div className="hi-workers__none">{L.loading}</div>}
          {shown.status === "failed" && <div className="hi-workers__none">{L.framesFailed}</div>}

          {shown.status === "ok" && onMessages && (
            messages.length === 0 ? (
              <div className="hi-workers__empty">
                <div className="hi-workers__empty-big">{L.noMessages}</div>
                {/* Frames on disk and nothing folded out of them is a *different* answer
                    from an empty session, and the reader must not blur the two: it means
                    this build could not read that log, and the wire tab still can. */}
                <div className="hi-workers__empty-sub">
                  {shown.frames ? L.noMessagesButFrames(shown.frames) : L.noFramesNote}
                </div>
              </div>
            ) : (
              <Transcript
                messages={messages}
                turns={shown.turns || []}
                openRow={openRow}
                setOpenRow={setOpenRow}
              />
            )
          )}

          {shown.status === "ok" && !onMessages && (
            frames.length === 0 ? (
              <div className="hi-workers__empty">
                <div className="hi-workers__empty-big">{L.noFrames}</div>
                <div className="hi-workers__empty-sub">{L.noFramesNote}</div>
              </div>
            ) : (
              frames.map((f, i) => (
                <Frame
                  key={i}
                  frame={f}
                  open={openRow === i}
                  onToggle={() => setOpenRow(openRow === i ? null : i)}
                />
              ))
            )
          )}
        </div>
      </div>
    </div>
  );
}

/** The messages, in wire order, with each turn drawn as a rule rather than a row.
 *
 *  A turn is the bracket around a stretch of the session, not a thing that happened in it —
 *  and the one a reader most wants to see is the turn that never closed, because that is
 *  where the session died. */
function Transcript({ messages, turns, openRow, setOpenRow }) {
  const byTurn = useMemo(() => {
    const out = [];
    for (const m of messages) {
      const last = out[out.length - 1];
      if (!last || last.turn !== m.turn) out.push({ turn: m.turn, rows: [m] });
      else last.rows.push(m);
    }
    return out;
  }, [messages]);

  return (
    <>
      {byTurn.map((g) => {
        const turn = turns.find((t) => t.n === g.turn);
        return (
          <div className="hi-workers__turn" key={`${g.turn}:${g.rows[0].seq}`}>
            {g.turn > 0 && (
              <div className="hi-workers__turn-rule" data-status={turn?.status}>
                <span>{L.turnN(g.turn)}</span>
                {turn?.tokens ? <span>{L.tokens(turn.tokens)}</span> : null}
                {turn && turn.status !== "completed" && (
                  <span className="is-warn">
                    {turn.status === "inProgress" ? L.turnOpen : turn.status}
                  </span>
                )}
              </div>
            )}
            {g.rows.map((m) => (
              <Msg
                key={`${m.seq}:${m.kind}`}
                m={m}
                open={openRow === m.seq}
                onToggle={() => setOpenRow(openRow === m.seq ? null : m.seq)}
              />
            ))}
          </div>
        );
      })}
    </>
  );
}

/** One message. The head is what it was; the body is the whole of it, on request.
 *
 *  Everything here collapses to a couple of lines and opens to its full text — a command's
 *  output and a tool call's arguments are routinely thousands of characters, and a
 *  transcript that pastes them inline is the wall this view exists to stop being. */
function Msg({ m, open, onToggle }) {
  const detail = body(m);
  const running = m.status && m.status !== "completed" && m.status !== "success";
  const kind = kindOf(m);
  return (
    <div className="hi-workers__msg" data-kind={kind} data-running={running ? "true" : undefined}>
      <button
        type="button"
        className="hi-workers__msg-head"
        aria-expanded={open}
        onClick={detail ? onToggle : undefined}
        data-flat={detail ? undefined : "true"}
      >
        <span className="hi-workers__msg-kind">{L.kind[kind] || m.kind}</span>
        <span className="hi-workers__msg-head-line">{head(m)}</span>
        <span className="hi-workers__msg-meta">
          {[
            m.kind === "command" && typeof m.exit === "number" ? L.exit(m.exit) : null,
            typeof m.ms === "number" ? L.tookMs(m.ms) : null,
            // A row still in progress at the end of the log is where it stopped, so the
            // word is kept rather than shown as a spinner that will never resolve.
            running ? m.status : null,
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
      </button>
      {open && detail && <pre className="hi-workers__msg-body">{detail}</pre>}
      {/* The peek is only for a body that says something the head does not — a command's
          output, a tool's arguments. On a text message the body *is* the head, longer, and
          a preview of it under it is the same sentence twice. */}
      {!open && detail && !SAYS_ITSELF.has(kind) && (
        <div className="hi-workers__msg-peek">{oneLine(detail)}</div>
      )}
    </div>
  );
}

/** Kinds whose whole body is the text already on the head line, at length. `say` is one:
 *  its head carries the utterance, and its body only adds what the call answered. */
const SAYS_ITSELF = new Set(["user", "agent", "say", "thinking", "warning", "stderr"]);

/** The row's own word for itself.
 *
 * `hi_say` is the one call in this panel that **is** speech: it is the whole way out to a
 * person, so its row says `said` and carries the words on its head line. Every other row,
 * the typed `agent` message included, is how the turn got there. Keeping the two apart is
 * the difference between a record and a claim — a log that labels typed prose `said` will
 * tell you the agent answered on a night the person heard nothing.
 */
function kindOf(m) {
  return m.kind === "tool" && m.tool === "hi_say" ? "say" : m.kind;
}

/** The one line that says what this message was. */
function head(m) {
  if (kindOf(m) === "say") return oneLine((m.arguments && m.arguments.text) || "");
  switch (m.kind) {
    case "command":
      return m.command || "";
    case "edit":
      // Basenames: the transcript is narrow and every path here shares a long prefix.
      return (m.paths || []).map((p) => String(p).split("/").pop()).join(", ");
    case "tool":
      return m.server ? `${m.server}/${m.tool}` : m.tool || "";
    case "search":
      return m.query || "";
    case "item":
      return m.type || "";
    default:
      return oneLine(m.text || "");
  }
}

/** Everything else there is of it — or `null` when the head already was the whole thing. */
function body(m) {
  switch (m.kind) {
    case "command":
      return m.output || null;
    case "edit":
      return m.diff || null;
    case "tool": {
      const parts = [];
      if (m.arguments !== undefined) parts.push(`${L.args}\n${pretty(JSON.stringify(m.arguments))}`);
      if (m.error) parts.push(`${L.errored}\n${pretty(JSON.stringify(m.error))}`);
      else if (m.result !== undefined) parts.push(`${L.result}\n${pretty(JSON.stringify(m.result))}`);
      return parts.join("\n\n") || null;
    }
    case "item":
      return pretty(JSON.stringify(m.item));
    default: {
      const text = m.text || "";
      // A message short enough to have been said in full by the head has no body — an
      // expander that reveals the line above it is a lie about there being more.
      return text.length > 160 || text.includes("\n") ? text : null;
    }
  }
}

function oneLine(s) {
  return String(s).replace(/\s+/g, " ").trim();
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
    /* One ended card, and therefore the band: it is exactly two of these plus the gap
       between them, which is only a height anyone can trust because every card in there is
       built to this number rather than to its own content. */
    --w-ended-h: 112px;
    --w-gap: 10px;
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
     is not a reset. It silently ate the card's padding and surface (cards rendered as bare
     text under a floating shadow, since box-shadow is the one thing it doesn't set), the
     close button's centring, and the frame head's monospace, via its font shorthand. It still
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

  .hi-workers__section-head {
    margin: 0 0 10px;
    font-size: 11.5px;
    font-weight: 700;
    letter-spacing: .08em;
    text-transform: uppercase;
    color: var(--fg-mute);
  }

  .hi-workers__section-head--ended {
    margin-top: 26px;
  }

  /* ── the tree ─────────────────────────────────────────────────────────────
     Sideways, not folded. A tree is as wide as the delegation is, and there is no
     honest narrower arrangement of one — wrapping a rank would draw a different tree.
     The plot centres inside this when it fits and scrolls when it doesn't.

     The reel around it carries the only mark of that scrolling anyone sees before they
     scroll: a fade on whichever edge has tree behind it. */
  .hi-workers__reel {
    position: relative;
    width: 100%;
  }

  .hi-workers__reel::before,
  .hi-workers__reel::after {
    content: "";
    position: absolute;
    top: 0;
    bottom: 6px;
    width: 44px;
    pointer-events: none;
    opacity: 0;
    transition: opacity 160ms var(--ease, ease);
  }

  .hi-workers__reel::before {
    left: 0;
    background: linear-gradient(to right, var(--paper), transparent);
  }

  .hi-workers__reel::after {
    right: 0;
    background: linear-gradient(to left, var(--paper), transparent);
  }

  .hi-workers__reel[data-more="left"]::before,
  .hi-workers__reel[data-more="both"]::before,
  .hi-workers__reel[data-more="right"]::after,
  .hi-workers__reel[data-more="both"]::after {
    opacity: 1;
  }

  .hi-workers__canvas {
    width: 100%;
    overflow-x: auto;
    overflow-y: hidden;
    padding-bottom: 6px;
  }

  /* The engine's coordinate space, and the only place on the page positioned in pixels.
     Cards are absolute inside it and the arrows are one SVG over the same box, so the two
     cannot drift apart. */
  .hi-workers__plot {
    position: relative;
    margin: 0 auto;
  }

  .hi-workers__wires {
    position: absolute;
    inset: 0;
    overflow: visible;
    pointer-events: none;
  }

  .hi-workers__wire {
    fill: none;
    stroke: var(--line-strong);
    stroke-width: 1.5;
    stroke-linecap: round;
  }

  /* Named tip, not head: hi-workers__head is the page's own header, and an SVG path that
     inherited its flexbox is a shape that never draws. */
  .hi-workers__tip {
    fill: var(--line-strong);
    stroke: none;
  }

  /* The open session's own arrows, so opening a card says where it sits without having to
     re-find it in the tree. */
  .hi-workers__wires g[data-lit="true"] .hi-workers__wire {
    stroke: var(--accent);
    stroke-width: 2;
  }

  .hi-workers__wires g[data-lit="true"] .hi-workers__tip {
    fill: var(--accent);
  }

  /* Positioned by the animator, in transform — never by CSS, and never with a CSS
     transition on top. The arrows are recomputed from these positions every frame, so a
     transition here that the animator did not run would slide the card out from under its
     own arrow. */
  .hi-workers__node {
    position: absolute;
    top: 0;
    left: 0;
    will-change: transform, opacity;
  }

  /* A card on its way out is still on screen and no longer a thing to click: its session
     is gone, and its address would read some other session under the same number. */
  .hi-workers__node[data-gone="true"] {
    pointer-events: none;
  }

  /* ── a card ───────────────────────────────────────────────────────────────
     A card, not a row. The difference is not the corner radius: a row puts every field
     on one line and buys the last one out of the first one's width, which is how a
     worker's task became four words and an ellipsis. A card gives each field its own
     line and lets the ones that are prose wrap. */
  .hi-workers__card {
    display: block;
    width: 100%;
    padding: 13px 15px 14px;
    background: var(--surface-strong);
    border-radius: 16px;
    box-shadow: var(--w-shadow);
    transition: background 120ms var(--ease, ease), box-shadow 120ms var(--ease, ease);
  }

  .hi-workers__card:hover {
    background: var(--surface);
  }

  .hi-workers__card[data-open="true"] {
    box-shadow: 0 0 0 2px var(--accent-line), var(--w-shadow);
  }

  /* An ended card is quieter than a live one by construction — flat, outlined, no lift. The
     page should read live-first without needing a legend to say so. */
  .hi-workers__card.is-ended {
    display: flex;
    flex-direction: column;
    height: var(--w-ended-h);
    padding: 10px 13px 11px;
    overflow: hidden;
    background: var(--surface);
    border: 1px solid var(--line);
    border-radius: 13px;
    box-shadow: none;
    animation: hi-workers-in 220ms var(--ease, ease);
  }

  /* Still running, owner gone — the dropped-report condition, named on the card rather
     than left as a tree that quietly grew a second root. */
  .hi-workers__orphan {
    display: block;
    margin-bottom: 7px;
    font-size: 11px;
    font-weight: 700;
    color: var(--danger);
  }

  /* The card's top strip. Two short things at opposite ends, and it still wraps: a card is
     narrow enough that "Reflection" and "ended 16h ago" do not always share a line, and
     wrapping is the right answer there rather than shrinking either. */
  .hi-workers__top {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 9px;
    min-width: 0;
  }

  /* And on an ended card it may not wrap, because that card's height is fixed: a strip
     that took a second line would push the title into the clip and leave a row of cards
     showing the top half of their own lettering. The elapsed figure gives way instead —
     it is the one thing on the card that is also in the tense of the heading above it. */
  .is-ended .hi-workers__top {
    flex-wrap: nowrap;
  }

  /* It may shrink, where the pill beside it may not: on a card whose strip cannot wrap,
     something has to give when the two do not fit, and a role read as "Task man…" is worse
     than a clock read as "cut off 1h a…". */
  .is-ended .hi-workers__state {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  /* The card's title. Two lines here and one on an ended card, and the difference is the
     width each has to work in: a card in the tree is as narrow as fitting the tree made it,
     which is narrow enough that one line of a written headline — "what reaches the person",
     "implement corrected KT8-059 padding" — stops in the middle of the subject. Two lines
     holds the whole of nearly every title the server sends. An ended card is wider (the
     band is full-frame and its cards are a grid) and is spending its height on being
     exactly two rows deep, so there one line is both enough and all there is.

     The clamp is a backstop either way: anything over the cap arrives already cut with an
     ellipsis. The brief still exists whole — it is the session's first prompt, one click
     away under "What happened". */
  .hi-workers__title {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    margin-top: 9px;
    min-width: 0;
    font-size: 14.5px;
    font-weight: 620;
    line-height: 1.35;
    letter-spacing: -.01em;
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  .is-ended .hi-workers__title {
    -webkit-line-clamp: 1;
    margin-top: 7px;
    font-size: 14px;
  }

  /* How it is, pushed to the strip's far end opposite the pill — and on an ended card, when
     it stopped. The same slot in both tenses, because they answer the same question. */
  .hi-workers__state {
    flex: none;
    margin-left: auto;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--fg-mute);
  }

  .hi-workers__card[data-state="running"] .hi-workers__state {
    color: var(--accent);
  }

  /* Work sitting in an inbox nobody has picked up. Not an error — a session mid-turn will
     take it next round — but it is the one state where a growing number means the drain
     has stopped, so it does not read as quiet. */
  .hi-workers__card[data-state="waiting"] .hi-workers__state {
    color: var(--accent-2);
  }

  .hi-workers__meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    margin-top: 7px;
    font-size: 12px;
    color: var(--fg-mute);
  }

  /* The ended card's meta: one clamped run of text, not a wrapping row of chips, so its
     height is two lines whatever it says. See the note beside it in EndedCard. */
  .hi-workers__meta.is-run-on {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    margin-top: 6px;
    line-height: 1.45;
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  .hi-workers__meta .is-warn {
    color: var(--danger);
    font-weight: 600;
  }

  /* Monospace, because it is nearly always a command or a tool name and proportional type
     makes those hard to scan. Two lines, not one: a card is a fixed width, and one line of
     mono at this size stops inside the shell invocation — before the command it is running,
     which is the only part worth reading. The panel still has the whole. */
  .hi-workers__doing {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    margin-top: 9px;
    font-family: var(--w-mono);
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--accent);
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  /* A turn that ended badly. Same slot and same clamp as the doing line — the two never
     appear together — but in the danger colour and proportional type: this is a sentence
     about what went wrong, not a command to scan. */
  .hi-workers__ended {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    margin-top: 9px;
    font-size: 12px;
    line-height: 1.45;
    font-weight: 600;
    color: var(--danger);
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  /* How long it has been on this one. Quieter than the line it qualifies — it is the
     second question, asked only once the first has an answer. */
  .hi-workers__age {
    color: var(--fg-mute);
  }

  /* What it said, clamped to two lines. On one line at this width it was about six words —
     not enough to tell two sessions apart, which is the only thing this line is here to
     do. Two rather than three, because every extra line a card can grow is a line the whole
     rank below it moves down. */
  .hi-workers__tail {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    margin-top: 7px;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--fg-dim);
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  /* ── what just ended ──────────────────────────────────────────────────────
     Full width under the tree, exactly two cards deep, and it scrolls. The depth is a
     promise the card height keeps: every card in here is --w-ended-h tall, so two rows
     is a number rather than an estimate. It contains its own overscroll, since the page
     itself scrolls and reaching the end of this band must not carry on into that. */
  .hi-workers__band {
    width: 100%;
    max-height: calc(var(--w-ended-h) * 2 + var(--w-gap));
    overflow-y: auto;
    overscroll-behavior: contain;
  }

  .hi-workers__band-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(228px, 1fr));
    grid-auto-rows: var(--w-ended-h);
    gap: var(--w-gap);
  }

  /* A card arriving, in the band and nowhere else — the tree's own arrivals are the
     animator's, since a card there also has to displace its neighbours. */
  @keyframes hi-workers-in {
    from { opacity: 0; transform: scale(.96); }
    to { opacity: 1; transform: none; }
  }

  @media (prefers-reduced-motion: reduce) {
    .hi-workers__card.is-ended { animation: none; }
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
     content inset and lie over the host's own surfaces — the controls, the conversation —
     which the plane model says an agent surface may never do (docs/arch/stage.md). */
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

  /* Two readings of one session, so they sit side by side rather than one being buried
     behind a menu: which one you want depends on whether you believe the fold. */
  .hi-workers__readings {
    display: flex;
    gap: 4px;
  }

  .hi-workers__reading {
    padding: 4px 10px;
    border-radius: 999px;
    font-size: 12.5px;
    font-weight: 650;
    color: var(--fg-mute);
  }

  .hi-workers__reading:hover {
    color: var(--fg);
  }

  .hi-workers__reading[data-on="true"] {
    color: var(--accent);
    background: var(--accent-wash);
  }

  /* ── the transcript ───────────────────────────────────────────────────────── */

  .hi-workers__turn + .hi-workers__turn {
    margin-top: 4px;
  }

  /* A rule, not a row: a turn is the bracket around a stretch of the session. */
  .hi-workers__turn-rule {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 14px 0 8px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: .06em;
    text-transform: uppercase;
    color: var(--fg-mute);
  }

  .hi-workers__turn-rule::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--line);
  }

  .hi-workers__turn-rule .is-warn {
    color: var(--danger);
  }

  /* The one a reader is looking for: the turn the session died inside. */
  .hi-workers__turn-rule[data-status="inProgress"],
  .hi-workers__turn-rule[data-status="failed"] {
    color: var(--danger);
  }

  .hi-workers__msg {
    padding: 6px 0;
    border-bottom: 1px solid var(--line);
  }

  .hi-workers__msg-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
    width: 100%;
  }

  .hi-workers__msg-head[data-flat="true"] {
    cursor: default;
  }

  /* A fixed column, so the kinds line up down the left edge and the shape of a turn —
     prompt, thought, ran, ran, ran, said — is legible without reading a word of it. */
  .hi-workers__msg-kind {
    flex: none;
    width: 64px;
    font-size: 11px;
    font-weight: 700;
    color: var(--fg-mute);
    text-align: right;
  }

  .hi-workers__msg-head-line {
    flex: 1;
    min-width: 0;
    font-size: 13px;
    line-height: 1.45;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hi-workers__msg-meta {
    flex: none;
    font-size: 11.5px;
    color: var(--fg-mute);
  }

  /* What the agent *said* is the point of the page; the rest — typed working-out
     included — is how it got there. The weight sat on the agent-message row back when
     that row was labelled "said", which put the emphasis on the line nobody heard. */
  .hi-workers__msg[data-kind="say"] .hi-workers__msg-head-line {
    font-weight: 620;
  }

  .hi-workers__msg[data-kind="user"] .hi-workers__msg-head-line {
    color: var(--fg-dim);
  }

  .hi-workers__msg[data-kind="thinking"] .hi-workers__msg-head-line {
    color: var(--fg-mute);
    font-style: italic;
  }

  .hi-workers__msg[data-kind="command"] .hi-workers__msg-head-line,
  .hi-workers__msg[data-kind="tool"] .hi-workers__msg-head-line,
  .hi-workers__msg[data-kind="edit"] .hi-workers__msg-head-line {
    font-family: var(--w-mono);
    font-size: 11.5px;
  }

  .hi-workers__msg[data-kind="tool"] .hi-workers__msg-head-line {
    color: var(--accent-2);
  }

  .hi-workers__msg[data-kind="warning"] .hi-workers__msg-head-line,
  .hi-workers__msg[data-kind="stderr"] .hi-workers__msg-head-line {
    color: var(--danger);
  }

  .hi-workers__msg[data-kind="stderr"] .hi-workers__msg-head-line {
    font-family: var(--w-mono);
    font-size: 11.5px;
  }

  /* Still running when the log ended — the row that says where it stopped. */
  .hi-workers__msg[data-running="true"] .hi-workers__msg-kind {
    color: var(--accent);
  }

  .hi-workers__msg-peek {
    margin: 3px 0 0 74px;
    font-family: var(--w-mono);
    font-size: 11px;
    line-height: 1.5;
    color: var(--fg-mute);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hi-workers__msg-body {
    margin: 6px 0 4px 74px;
    padding: 10px 12px;
    max-height: 46vh;
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
