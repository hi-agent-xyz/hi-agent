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
//    Cognition had three sessions out. Workers nest under their owner, one indent, one
//    level deep.
//
//    That nest sits in one of **three columns** — the outward ladder, Reflection, and what
//    just ended, at 4 · 4 · 2 — rather than in one long page of full-width rows. A row
//    spent the frame's whole width on a line that was mostly empty and then bought the
//    title out of what was left. A column-width card gives each field its own line instead.
//    The frame here is the window *minus* the conversation rail (`docs/arch/stage.md`,
//    ~320-460px), which is what the columns fold on: three, then two, then one, off a
//    container query, because the rail changes this frame without changing the window.
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
// 3. **Doing, not just said.** `tail` is the session's own words, and a worker grinding
//    through shell commands says nothing for minutes — so a blank row read as a dead one.
//    `doing` is the other half, and the two never share a line.
// 4. **A row has one state, and which fields are meaningful is a function of it.** The
//    three above are each a field earning its place; this one is about what happens when
//    several of them are drawn as though they were independent. They were, and the row
//    contradicted itself three separate times — see `LiveRow`. The server now sends one
//    `state` word and this file gates on it.
//
// Clicking any row — live or ended — opens what that session did. Those frames have been
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
// Colour comes from the host theme tokens (see tasks.jsx for the vocabulary). Polls, because
// the whole value is that it is current.
import { useState, useEffect, useCallback, useRef, useMemo } from "react";

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
    lostToRestart: "lost to a restart",
    lostNote: "The process ended underneath it — nothing got to report what it found.",
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
    // `agent` is **typed**, never "said". An `agentMessage` is the model's own
    // working-out and reaches nobody; the only thing a person ever heard is a `hi_say`
    // tool call, which folds as `tool`. Labelling it "said" told readers the agent had
    // answered when the person got nothing — the one row in this table that can make a
    // record contradict what happened.
    kind: {
      user: "prompt",
      agent: "typed",
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
    turns: (n) => `${n} 轮`,
    up: (t) => `已运行 ${t}`,
    live: "在跑", endedHead: "刚结束",
    orphans: "还在跑,派它的已经没了",
    orphanNote: (id) => `派它的 ${id} 已经不在了`,
    onTask: (subject) => `做 ${subject}`,
    unlinked: "没挂到任何任务",
    ownedBy: (id) => `归 ${id}`,
    endedAgo: (t) => `${t}前结束`,
    lostToRestart: "重启时丢了",
    lostNote: "进程在它下面结束了 —— 它做了什么没来得及报。",
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
      agent: "写",
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

/** The ladder, top to bottom — the order `docs/arch/agents.md` gives: the voice, the
 *  thinking behind it, the outward brain, the housekeeper. A role not named here sorts
 *  after these rather than being dropped, so a sixth rung appears instead of vanishing. */
const LADDER = ["reaction", "deliberation", "cognition", "reflection"];

/** What each rung and each kind of worker is **called** — `docs/arch/agents.md`, the same
 *  words as the `role` field this page reads and the `X-HI-Role` header the sessions
 *  themselves carry.
 *
 *  These used to be descriptions of what each rung *does*: `speaking`, `mulling`,
 *  `thinking`, `filing`. Two things were wrong with that, and both showed up on screen at
 *  once. A present participle reads as a live status, and the actual status sits two lines
 *  below it — so a row said `speaking` and then `idle`, which is a contradiction if you
 *  read the pill as it is written. And the task line beside it already names the rung, and
 *  names it better: `speaking · the voice`, `thinking · the shared brain`. The pill was
 *  saying the same thing twice, in the worse of the two words.
 *
 *  Not translated, in either direction. A rung's name is this system's own vocabulary
 *  (`views/builtin.rs`, rule 3) — it names a part of this architecture, not an ordinary
 *  object — so there is one table here rather than one per language. */
const ROLE = {
  reaction: "Reaction",
  deliberation: "Deliberation",
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
  "file-filer": "File filer",
  "person-reader": "Person reader",
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
  const [outward, inward] = split(groups);

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

        {/* Three columns, and a card's column is a function of what it is — see `split`.
            Each column is its own stack, so a worker appearing under Cognition two
            seconds from now lengthens one column instead of re-flowing the page. */}
        <div className="hi-workers__cols">
          <h2 className="hi-workers__section-head hi-workers__col-head--live">{L.live}</h2>
          <h2 className="hi-workers__section-head hi-workers__col-head--ended">{L.endedHead}</h2>

          {running === 0 ? (
            <div className="hi-workers__empty">
              <div className="hi-workers__empty-big">{L.emptyBig}</div>
              <div className="hi-workers__empty-sub">{L.emptySub}</div>
            </div>
          ) : (
            <>
              <div className="hi-workers__col hi-workers__col--outward">
                {outward.map((g) => (
                  <Group key={g.key} group={g} open={open} setOpen={setOpen} />
                ))}
              </div>
              <div className="hi-workers__col hi-workers__col--inward">
                {inward.map((g) => (
                  <Group key={g.key} group={g} open={open} setOpen={setOpen} />
                ))}
              </div>
            </>
          )}

          <div className="hi-workers__col hi-workers__col--ended">
            {ended.length === 0 ? (
              <div className="hi-workers__none">{L.noEnded}</div>
            ) : (
              ended.map((e) => (
                <EndedRow key={`${e.run}:${e.session}`} row={e} open={open} setOpen={setOpen} />
              ))
            )}
          </div>
        </div>
      </div>

      {open && <Session addr={open} onClose={() => setOpen(null)} />}
    </div>
  );
}

/** Which live column a group belongs in.
 *
 *  By what the rung is *for*, not by how many there are. The outward ladder — the
 *  voice, the thinking behind it, the outward brain — is what someone opens this
 *  page to watch, so it takes the first column. Reflection is the housekeeping rung
 *  and takes the second, so its workers (a filer, a view-builder) can never push the
 *  outward ladder down the page.
 *
 *  A rung this file has never heard of lands in the outward column rather than
 *  nowhere — the same rule `tree` follows for an unknown role, and for the same
 *  reason: a live session missing from this page is the one failure it cannot have.
 *  Orphans keep that placement too, since their warning belongs where the eye is.
 */
const INWARD = new Set(["reflection"]);

function split(groups) {
  const outward = [];
  const inward = [];
  for (const g of groups) (g.owner && INWARD.has(g.owner.role) ? inward : outward).push(g);
  return [outward, inward];
}

/** An owner and what it spawned — the unit that sits in a column. */
function Group({ group, open, setOpen }) {
  return (
    <div className="hi-workers__group">
      {group.orphaned && <div className="hi-workers__group-note">{L.orphans}</div>}
      {group.owner && <LiveRow row={group.owner} open={open} setOpen={setOpen} />}
      {group.children.length > 0 && (
        <div className="hi-workers__kids" data-rooted={group.owner ? "true" : undefined}>
          {group.children.map((c) => (
            <LiveRow key={c.id} row={c} open={open} setOpen={setOpen} indent />
          ))}
        </div>
      )}
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

/** One live session.
 *
 *  **A row has one state, and which fields are meaningful is a function of it.** That rule
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
function LiveRow({ row, open, setOpen, indent }) {
  // A live row is addressed by id with no run, so it cannot collide with an ended row that
  // happens to share the number — which, across runs, they routinely do.
  const isOpen = !!open && open.id === row.id && !open.run;
  // An unknown state word from a newer server reads as idle rather than blanking the row:
  // the roster showing a session at all is the load-bearing part.
  const state = L.state[row.state] ? row.state : "idle";
  const running = state === "running";
  return (
    <button
      type="button"
      className="hi-workers__row"
      data-indent={indent ? "true" : undefined}
      data-open={isOpen ? "true" : undefined}
      data-state={state}
      aria-expanded={isOpen}
      onClick={() => setOpen(isOpen ? null : { id: row.id, run: null })}
    >
      {/* The card's top strip: what this is, and how it is. The two facts that are true
          of the session as a whole, in a fixed place on every card, so a column of them
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
          nothing gained by giving it two lines — a column of cards whose titles are all one
          line is the thing that actually scans. Anything over the cap arrives already cut
          with an ellipsis; the CSS clamp is the backstop for a wide character set. */}
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
        {/* Only once the owner has gone. While it is live the indent already shows who it
            is, and repeating it on every child is noise. */}
        {row.owner && !row.owner_role && (
          <span className="is-warn">{L.orphanNote(row.owner)}</span>
        )}
      </span>

      {/* Doing and said are different questions and never share a line. A row with a
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
      <span className="hi-workers__top">
        <span className="hi-workers__dot" aria-hidden />
        <span className="hi-workers__pill">{label(row)}</span>
        <span className="hi-workers__elapsed">
          {lost ? L.lostToRestart : when ? L.endedAgo(elapsed(when)) : ""}
        </span>
      </span>

      <span className="hi-workers__title">{row.title || L.noTitle}</span>

      <span className="hi-workers__meta">
        {typeof row.turns === "number" && <span>{L.turns(row.turns)}</span>}
        {row.started && row.ended && <span>{L.ranFor(between(row.started, row.ended))}</span>}
        {row.subject && <span>{L.onTask(row.subject)}</span>}
        {row.owner && <span>{L.ownedBy(row.owner)}</span>}
      </span>

      {/* Said plainly, because "it was cut off" and "it finished" are not the same outcome,
          and the difference is the reason this row exists at all. */}
      {lost && <span className="hi-workers__lost">{L.lostNote}</span>}
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
              <Said
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
function Said({ m, open, onToggle }) {
  const detail = body(m);
  const running = m.status && m.status !== "completed" && m.status !== "success";
  return (
    <div className="hi-workers__msg" data-kind={m.kind} data-running={running ? "true" : undefined}>
      <button
        type="button"
        className="hi-workers__msg-head"
        aria-expanded={open}
        onClick={detail ? onToggle : undefined}
        data-flat={detail ? undefined : "true"}
      >
        <span className="hi-workers__msg-kind">{L.kind[m.kind] || m.kind}</span>
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
      {!open && detail && !SAYS_ITSELF.has(m.kind) && (
        <div className="hi-workers__msg-peek">{oneLine(detail)}</div>
      )}
    </div>
  );
}

/** Kinds whose whole body is the text already on the head line, at length. */
const SAYS_ITSELF = new Set(["user", "agent", "thinking", "warning", "stderr"]);

/** The one line that says what this message was. */
function head(m) {
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
    /* The columns below fold on the width of *this frame*, not the window's. The two
       are different by a rail: the same 1200px window is a 1200px frame with the
       conversation collapsed and an ~800px one with it open, and a media query cannot
       tell those apart. It is already the containing block (position: relative), so
       inline-size containment adds no layout the panel depends on. */
    container-type: inline-size;
    container-name: workers;
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

  /* ── the three columns ────────────────────────────────────────────────────
     4 · 4 · 2. The outward ladder and the housekeeping one get equal width because
     both hold cards with a task paragraph on them; the ended column holds a title and
     two chips, so it takes half of one of those and gives the width back.

     Every item is placed explicitly — row and column — rather than flowed. With
     auto-placement the "Just ended" heading lands wherever the item before it left
     off, which is beside the live cards at one width and above them at another; the
     heading of a column must sit on the column.

     A short column simply ends: align-items:start so a stack of two does not
     stretch to the height of a stack of six, and nothing re-orders when it grows. */
  .hi-workers__cols {
    display: grid;
    grid-template-columns: 4fr 4fr 2fr;
    align-items: start;
    gap: 10px 18px;
  }

  .hi-workers__section-head {
    margin: 0 0 4px;
    font-size: 11.5px;
    font-weight: 700;
    letter-spacing: .08em;
    text-transform: uppercase;
    color: var(--fg-mute);
  }

  .hi-workers__col-head--live { grid-row: 1; grid-column: 1 / 3; }
  .hi-workers__col-head--ended { grid-row: 1; grid-column: 3; }

  .hi-workers__col {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    grid-row: 2;
  }

  .hi-workers__col--outward { grid-column: 1; }
  .hi-workers__col--inward { grid-column: 2; }
  .hi-workers__col--ended { grid-column: 3; }

  /* Nothing live: the answer takes both live columns rather than sitting in the
     first one with an empty column beside it. */
  .hi-workers__empty {
    grid-row: 2;
    grid-column: 1 / 3;
  }

  /* One column is not enough for three of them side by side — a card whose title is a
     paragraph needs roughly 280px before it reads as a card at all. The ended column
     goes under the two live ones first, since it is the one nobody opened this page
     for; below that, everything stacks. */
  @container workers (max-width: 900px) {
    .hi-workers__cols { grid-template-columns: 1fr 1fr; }
    .hi-workers__col-head--live { grid-row: 1; grid-column: 1 / 3; }
    .hi-workers__col--outward { grid-row: 2; grid-column: 1; }
    .hi-workers__col--inward { grid-row: 2; grid-column: 2; }
    .hi-workers__empty { grid-row: 2; grid-column: 1 / 3; }
    .hi-workers__col-head--ended { grid-row: 3; grid-column: 1 / 3; margin-top: 12px; }
    .hi-workers__col--ended { grid-row: 4; grid-column: 1 / 3; }
  }

  @container workers (max-width: 560px) {
    .hi-workers__cols { grid-template-columns: 1fr; }
    .hi-workers__col-head--live,
    .hi-workers__col--outward,
    .hi-workers__col--inward,
    .hi-workers__empty,
    .hi-workers__col-head--ended,
    .hi-workers__col--ended { grid-column: 1; }
    .hi-workers__col-head--live { grid-row: 1; }
    .hi-workers__col--outward { grid-row: 2; }
    .hi-workers__empty { grid-row: 2; }
    .hi-workers__col--inward { grid-row: 3; }
    .hi-workers__col-head--ended { grid-row: 4; margin-top: 12px; }
    .hi-workers__col--ended { grid-row: 5; }
  }

  .hi-workers__group-note {
    padding: 0 2px 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--danger);
  }

  /* One indent, and a rule to carry the eye from an owner down to what it spawned. A
     column is a third of the frame, so the inset is smaller than it would be across the
     page and the nesting stays one level only — depth would cost width the task needs. */
  .hi-workers__kids {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }

  .hi-workers__kids[data-rooted="true"] {
    margin-left: 10px;
    padding-left: 10px;
    border-left: 2px solid var(--line);
  }

  /* A card, not a row. The difference is not the corner radius: a row puts every field
     on one line and buys the last one out of the first one's width, which is how a
     worker's task became four words and an ellipsis. A card gives each field its own
     line and lets the ones that are prose wrap. */
  .hi-workers__row {
    display: block;
    width: 100%;
    padding: 13px 15px 14px;
    background: var(--surface-strong);
    border-radius: 16px;
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

  .hi-workers__row.is-ended[data-lost="true"] {
    border-color: var(--danger-line);
    background: var(--danger-wash);
  }

  /* The card's top strip. Two short things at opposite ends, and it still wraps: the
     ended column is a fifth of the frame, narrow enough that "Reflection" and
     "ended 16h ago" do not always share a line, and wrapping is the right answer there
     rather than shrinking either. */
  .hi-workers__top {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 9px;
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

  /* The card's title, on its own line and held to one. It used to wrap to two, because
     what the server sent was the paragraph of instruction the worker was briefed with and
     two lines was the least-bad amount of it to show. The server sends a written headline
     now, so one line is the whole thing rather than a fragment of it, and a column of
     one-line titles is what makes a stack of cards scannable. The brief still exists — it
     is the session's first prompt, whole, one click away under "What happened". */
  .hi-workers__title {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 1;
    margin-top: 9px;
    min-width: 0;
    font-size: 14.5px;
    font-weight: 620;
    line-height: 1.35;
    letter-spacing: -.01em;
    overflow: hidden;
    overflow-wrap: anywhere;
  }

  /* Pushed to the strip's far end, opposite the pill — the same slot the live card's
     state word holds, because they answer the same question one tense apart. */
  .hi-workers__elapsed {
    flex: none;
    margin-left: auto;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--fg-mute);
  }

  /* The live card's answer. Pushed right so it stays on the card's edge when the strip
     wraps. */
  .hi-workers__state {
    flex: none;
    margin-left: auto;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--fg-mute);
  }

  .hi-workers__row[data-state="running"] .hi-workers__state {
    color: var(--accent);
  }

  /* Work sitting in an inbox nobody has picked up. Not an error — a session mid-turn will
     take it next round — but it is the one state where a growing number means the drain
     has stopped, so it does not read as quiet. */
  .hi-workers__row[data-state="waiting"] .hi-workers__state {
    color: var(--accent-2);
  }

  .hi-workers__meta {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    margin-top: 7px;
    font-size: 12px;
    color: var(--fg-mute);
  }

  .hi-workers__meta .is-warn {
    color: var(--danger);
    font-weight: 600;
  }

  /* Monospace, because it is nearly always a command or a tool name and proportional type
     makes those hard to scan. Two lines, not one: a card is a third of the frame wide, and
     one line of mono at this size stops inside the shell invocation — before the command
     it is running, which is the only part worth reading. The panel still has the whole. */
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

  /* How long it has been on this one. Quieter than the line it qualifies — it is the
     second question, asked only once the first has an answer. */
  .hi-workers__age {
    color: var(--fg-mute);
  }

  /* What it said, clamped to three lines. On one line in a column this wide it was
     about six words — not enough to tell two sessions apart, which is the only thing
     this line is here to do. */
  .hi-workers__tail {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    margin-top: 7px;
    font-size: 12.5px;
    line-height: 1.5;
    color: var(--fg-dim);
    overflow: hidden;
    overflow-wrap: anywhere;
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

  /* What the agent said is the point of the page; the rest is how it got there. */
  .hi-workers__msg[data-kind="agent"] .hi-workers__msg-head-line {
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
