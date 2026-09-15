// purpose: Home is what is going on right now — the conversation, the work in hand, and
// nothing else. This factory view is compiled as a single JSX module, without bundling
// local imports. Keep the pure read model above Home; home.test.mjs tests it without React.
//
// **What this surface is NOT is the point of it.** It used to draw the retention policy:
// every task closed in a window, every session that had ever ended in it, and every result
// file any of them produced, each as a peer card on one canvas. Measured on a real instance
// that was 225 cards over 2556x16529px, of which the work actually in hand — ten open tasks
// and ten live sessions — was 8.4%. Result files alone were 60.2%. You opened Home and saw
// two cards out of two hundred, and neither was the point.
//
// So completeness moved out. `factory/tasks` already carries the whole ledger (and already
// learned this lesson: "closed work is not hidden, it is *thin*"), and `factory/workers`
// carries every session. Home does not repeat them. It shows what is open, what is live,
// and what we are talking about, and hands off for anything else.
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useLive, useWatched, useMessages, useViews, TEMPO } from "@hi/core";
import { flextree } from "d3-flextree";

const COPY = {
  en: {
    core: "Hi Agent", context: "Our conversation", update: "Latest update",
    nothing: "Nothing in hand", reading: "Loading work...",
    failed: "Some sources could not be refreshed", retry: "Retry", stale: "Earlier context",
    unknown: "Time unknown", untitled: "Untitled activity", noMessages: "No conversation yet",
    status: { todo: "To do", doing: "In progress", serving: "On duty", done: "Completed", cancelled: "Cancelled",
      running: "Working", waiting: "Work queued", idle: "Idle",
      failed: "Last turn failed", interrupted: "Last turn interrupted", missing: "Not connected" },
    roles: { reaction: "Conversation", cognition: "Coordination", reflection: "Review" },
    source: { tasks: "tasks", workers: "live sessions", views: "results" },
    ago: (n, unit) => `${n}${unit} ago`,
  },
  zh: {
    core: "Hi Agent", context: "我们的交流", update: "最新更新",
    nothing: "手头没有在办的事", reading: "正在读取工作...",
    failed: "部分数据未能刷新", retry: "重试", stale: "较早的上下文",
    unknown: "时间未知", untitled: "未命名活动", noMessages: "还没有对话",
    status: { todo: "待开始", doing: "进行中", serving: "值守", done: "已完成", cancelled: "已取消",
      running: "正在处理", waiting: "有工作待处理", idle: "空闲",
      failed: "上一轮失败", interrupted: "上一轮中断", missing: "未连接" },
    roles: { reaction: "交流", cognition: "协调", reflection: "回顾" },
    source: { tasks: "任务", workers: "在线会话", views: "成果" },
    ago: (n, unit) => `${n}${{ m: "分钟", h: "小时", d: "天" }[unit]}前`,
  },
};
const L = typeof document !== "undefined" && /^zh/i.test(document.documentElement.lang || navigator.language)
  ? COPY.zh : COPY.en;
const WINDOW_MS = 24 * 3600000;
const CORE_ROLES = new Set(["reaction", "cognition", "reflection"]);
const OPEN = new Set(["todo", "doing", "serving"]);
/**
 * How many pictures a task hangs below itself.
 *
 * **This is not the old artifact rank coming back.** That drew every result as a 280x134
 * peer card — 125 of them, 106 nothing but a filename, 60.2% of the canvas. What returns is
 * strictly the pictures, strictly below their task, as image tiles with no text, and capped:
 * a task that produces forty screenshots hangs six. The rest, and every result with no
 * picture, are `factory/tasks`' to show — Home carries no count of them.
 */
const RESULT_TILES = 6;
/** Where a card hands off. Home owns no detail of its own. */
const TASK_BOARD = "factory/tasks", SESSION_BOARD = "factory/workers";
const TONE = { core: "var(--accent)", todo: "var(--fg-mute)",
  doing: "var(--accent)", serving: "var(--accent-2)", done: "var(--fg-mute)", cancelled: "var(--fg-mute)",
  running: "var(--accent)", waiting: "var(--accent-2)", idle: "var(--fg-mute)",
  failed: "var(--danger)", interrupted: "var(--danger)", overview: "var(--accent)",
  result: "var(--accent-2)" };

/**
 * HomeModel is a read projection, NEVER another task/session lifecycle store.
 *
 * HomeNode = { id, kind, title, summary?, sourceRefs: SourceRef[], data: NodeData }
 * NodeData is discriminated by kind:
 * - core: { sessions: SessionState[], overviewIds: string[] }
 * - task: { task: TaskDto, status, endedAt, results: Result[] }
 * - activity: { session: SessionState, taskId?, ownerSessionId?, currentAction? }
 * - overview: { category, text, updatedAt, freshness, relatedNodeIds }
 * HomeEdge = { id, from, to, relation, primary }
 * Every non-root node has ONE primary parent; non-primary edges reference an existing
 * object without duplicating it. Nodes have no x/y or selected properties.
 *
 * Internal mappings:
 * 1. TaskDto (/api/tasks) -> task:<subject>. Status/time are copied, not inferred from
 *    sessions. Open tasks always; a closed one only while its own closure is recent.
 * 2. Registry Status (/api/workers) -> session:<run>:<id>. running means busy; waiting
 *    means QUEUED WORK, not waiting for the user; idle is still a LIVE session. Only live
 *    sessions are here at all — a session that has ended is `factory/workers`' subject.
 * 3. Reaction/Cognition/Reflection sessions compose the one core. All other live sessions
 *    become activities, even without a task.
 * 4. subject is the authoritative activity -> task join. owner is a technical session
 *    relationship only; it must not pretend that two independent tasks are one piece of work.
 * 5. Overview uses useMessages()'s USER-VISIBLE transcript and factual task transitions.
 *    No registry tail, raw reasoning, or tool log is used to manufacture a public plan.
 * 6. Every task is its own branch off the core. There is no grouping rank — see below.
 * 7. Only a task's PICTURES are nodes, capped. Results as cards were 60.2% of the canvas.
 *
 * A view is a single-file transform. Pure model/layout functions stay here rather than
 * importing an unserved local module. The tests evaluate only this section.
 */
const instant = (value) => {
  const n = value ? Date.parse(value) : NaN;
  return Number.isFinite(n) ? n : null;
};
/**
 * **An unknown closure time means old, not timeless.**
 *
 * This used to read `instant(value) === null || ...` — no stamp, so keep it, on the rule
 * that a missing time must not be fabricated. Not fabricating it is right; concluding that
 * the record is therefore current is not, and it is the second half that ran. On the
 * instance that exposed this, nine closed tasks carried no closure stamp at all and sat on
 * the canvas permanently; their records had last been touched over a month earlier. Of the
 * twenty-five closed tasks drawn, exactly one was there because it closed inside the window.
 *
 * "We cannot say when it closed" and "we can say it is not recent" are different claims.
 * The card may still print `Time unknown`; it may not use that to outlive the window.
 */
const recent = (value, now) => instant(value) !== null && instant(value) >= now - WINDOW_MS;
const taskEnd = (task) => task.status === "done" ? task.completedAt || task.statusSince || null
  : task.status === "cancelled" ? task.cancelledAt || task.statusSince || null : null;
const taskKey = (subject) => `task:${subject}`;
const sessionKey = (s) => `session:${s.run}:${s.id || s.session}`;
const plain = (value) => String(value || "").replace(/\s+/g, " ").trim();
const excerpt = (value, max = 180) => {
  const text = plain(value);
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
};
const ref = (kind, id) => ({ kind, id });

function normalizeSession(raw) {
  return {
    id: sessionKey(raw), run: raw.run, slug: raw.id || raw.session, role: raw.role,
    title: plain(raw.title) || L.untitled, online: true,
    state: raw.state, startedAt: raw.started || null, stateSince: raw.state_since,
    subject: raw.subject || null,
    ownerSessionId: raw.owner ? sessionKey({ run: raw.run, id: raw.owner }) : null,
    // doing survives a turn in the registry; never present stale work as current.
    currentAction: raw.state === "running" && raw.doing
      ? { text: raw.doing, at: raw.doing_at || null } : null,
    lastTurn: raw.last_turn || null,
  };
}

function taskResults(task, views) {
  // The row names what it mentions and this decides what that means. Matching moved to the
  // server (`view_refs` in `foundation/server/tasks.rs`) when the row stopped carrying the
  // prose to match against — same four spellings, and the same rule that only a known view
  // can be opened. What changed here is what a match becomes: a picture, or nothing.
  //
  // **A system view is never a task's product.** A ref is a MENTION, and a task whose prose
  // says "looked at factory/home" gets `factory/home` in its refs; three such mentions were
  // being taken as results across the work in hand, and two of them were being drawn as
  // the task's picture — a task about a birthday deck showed this very surface. The app's
  // own surfaces are the one class of mention that is categorically not an output.
  //
  // It does not fix a mention of ANOTHER task's result, which is the same confusion without
  // a flag to catch it: `research-two-pairs` is currently a result of the unrelated KTV
  // task. That belongs to the server-side matching, not here.
  //
  // **Only a picture is kept.** A result without one — a view never shot, a file the task
  // wrote — used to feed a count printed on the card, and the count is gone: a number with
  // nothing to open behind it is a claim the card cannot back, and `factory/tasks` lists them.
  const mentioned = new Set(task.refs || []);
  return views.filter((v) => v.view_ref && !v.system && v.shot_url && mentioned.has(v.view_ref))
    .map((v) => ({ id: v.view_ref, title: v.label || v.view_ref, shot: v.shot_url }));
}

/**
 * Every picture a task has is an image node below it, and a card never wears one.
 *
 * A card used to carry the first picture inside itself, beside its text, and hang the rest as
 * tiles. That made two appearances for one thing: the same kind of record was a strip inside
 * a bordered card here and a bare image tile there, and which one it got depended on nothing
 * the person can see — only on whether it happened to be first in `refs`. On the canvas it
 * read as clutter rather than as rank. One appearance for pictures, one for cards.
 *
 * A result with no picture is never a node: twenty-one liveness JSONs are a log, and
 * twenty-one cards for them were 45% of the old canvas. The pictures below say what the work
 * actually made; what it wrote besides is `factory/tasks`' to list.
 *
 * **Order still decides which six, and it is not "the latest".** View records have no
 * timestamp; the only time-like thing on them is the `?v=` cache-buster on the shot URL, which
 * is not declared to mean recency and would break silently the day it became a hash. The tiles
 * are the first shot-bearing views in the task's own `refs` order, and claim nothing more.
 */

function buildHome({ tasks = [], workers = [], views = [], messages = [] }, now = Date.now()) {
  const nodes = [];
  const edges = [];
  const edgeIds = new Set();
  const byId = new Map();
  const add = (node) => {
    if (byId.has(node.id)) return byId.get(node.id);
    nodes.push(node); byId.set(node.id, node); return node;
  };
  const link = (from, to, relation = "contains", primary = true) => {
    const id = `${from}/${relation}/${to}`;
    if (!edgeIds.has(id)) { edgeIds.add(id); edges.push({ id, from, to, relation, primary }); }
  };
  const sessions = new Map();
  for (const s of workers) sessions.set(sessionKey(s), normalizeSession(s));
  const coreSessions = [...sessions.values()].filter((s) => CORE_ROLES.has(s.role));
  const activities = [...sessions.values()].filter((s) => !CORE_ROLES.has(s.role));
  const core = add({ id: "core", kind: "core", title: L.core,
    sourceRefs: coreSessions.map((s) => ref("session", s.id)), data: { sessions: coreSessions, overviewIds: [] } });
  for (const task of tasks) {
    // **A live session no longer re-admits its expired task.** That rule kept a closed task
    // present "as context" whenever anything recent still named it, and it was the single
    // biggest leak: fifteen of the twenty-five closed tasks on the canvas arrived that way,
    // the oldest closed twenty-six days earlier, each drawn as a peer of the work in hand.
    // A session whose task has aged out now connects to the core and keeps its own title.
    if (!OPEN.has(task.status) && !recent(taskEnd(task), now)) continue;
    const node = add({ id: taskKey(task.subject), kind: "task", title: task.title || task.subject,
      sourceRefs: [ref("task", task.subject)], data: { task, status: task.status,
        endedAt: taskEnd(task), results: taskResults(task, views) } });
    // **Every task is its own branch off the core, and nothing groups them.** There used to be
    // a topic rank, named by the task's `project` or, failing that, its `systems`. No writer
    // has ever produced a `project`, so every topic drawn was a `systems` value — and `systems`
    // says which operational records a task touches, not what it belongs to. A birthday deck
    // whose photos arrived over Feishu sat under a "feishu" topic beside a client brief. A
    // grouping is a claim someone has to make; until something writes one, there is none.
    link("core", node.id);
    // Pictures are second-level, the way a sub-step or a sub-result is — all of them, so that
    // one kind of record has one appearance. A task mid-flight often has process pictures and
    // no deliverable yet, and nothing here claims to know which of them is which.
    for (const result of node.data.results.slice(0, RESULT_TILES)) {
      const id = `result:${result.id}`;
      const exists = byId.has(id);
      add({ id, kind: "result", title: result.title, sourceRefs: [ref("view", result.id)],
        data: { viewRef: result.id, shot: result.shot } });
      link(node.id, id, "produces", !exists);
    }
  }
  for (const session of activities) {
    const taskId = session.subject && byId.has(taskKey(session.subject)) ? taskKey(session.subject) : null;
    add({ id: session.id, kind: "activity", title: session.title, sourceRefs: [ref("session", session.id)],
      data: { session, taskId, ownerSessionId: session.ownerSessionId, currentAction: session.currentAction } });
    link(taskId || "core", session.id, taskId ? "works-on" : "contains");
  }
  const publicMessages = messages.filter((m) => (m.role === "user" || m.role === "agent") && plain(m.text));
  const lastUser = [...publicMessages].reverse().find((m) => m.role === "user");
  const lastAgent = [...publicMessages].reverse().find((m) => m.role === "agent");
  for (const message of [lastUser, lastAgent].filter(Boolean)) {
    const id = `overview:message:${message.id}`;
    const category = message.role === "user" ? "context" : "update";
    const stale = message.role === "agent" && lastUser && publicMessages.indexOf(message) < publicMessages.indexOf(lastUser);
    add({ id, kind: "overview", title: L[category], summary: excerpt(message.text),
      sourceRefs: [ref("message", message.id)], data: { category, text: message.text, updatedAt: message.ts,
        freshness: stale ? "stale" : "current", relatedNodeIds: [] } });
    link("core", id, "explains"); core.data.overviewIds.push(id);
  }
  // Factual transitions are safe to summarize without promoting worker prose into a
  // public statement. They remain individually addressable and link to the original task.
  // These now report only genuinely recent closures — which is the honest count, and on the
  // instance this was written against took the list from ten entries to one.
  for (const node of [...nodes]) {
    if (node.kind !== "task" || OPEN.has(node.data.status)) continue;
    const id = `overview:task:${node.id}`;
    add({ id, kind: "overview", title: node.title,
      summary: L.status[node.data.status], sourceRefs: node.sourceRefs,
      data: { category: "update", text: `${node.title} · ${L.status[node.data.status]}`,
        updatedAt: node.data.endedAt, freshness: "current", relatedNodeIds: [node.id] } });
    link("core", id, "explains"); link(id, node.id, "explains", false);
    core.data.overviewIds.push(id);
  }
  return { rootId: "core", asOf: new Date(now).toISOString(), nodes, edges };
}

function childIndex(model) {
  const children = new Map(model.nodes.map((n) => [n.id, []]));
  const nodes = new Map(model.nodes.map((n) => [n.id, n]));
  for (const edge of model.edges) if (edge.primary && nodes.has(edge.to)) children.get(edge.from)?.push(nodes.get(edge.to));
  return children;
}

function stateOf(node) {
  if (node.kind === "task") return node.data.status;
  if (node.kind !== "activity") return node.kind;
  const s = node.data.session;
  if (s.state !== "running" && ["failed", "interrupted"].includes(s.lastTurn?.outcome)) return s.lastTurn.outcome;
  return s.state;
}

function age(value, now) {
  const at = instant(value);
  if (at === null) return L.unknown;
  const minutes = Math.max(0, Math.floor((now - at) / 60000));
  return minutes < 60 ? L.ago(minutes, "m") : minutes < 1440 ? L.ago(Math.floor(minutes / 60), "h") : L.ago(Math.floor(minutes / 1440), "d");
}

function nodeTime(node) {
  return node.kind === "task" ? node.data.endedAt || node.data.task.statusSince
    : node.kind === "activity" ? node.data.session.stateSince
    : node.kind === "overview" ? node.data.updatedAt : null;
}

function emphasis(node, now) {
  const end = node.kind === "task" ? node.data.endedAt : null;
  if (instant(end) === null) return 1;
  // Fade emphasis, not legibility: even at 24h the card and its wire remain readable.
  return 1 - 0.22 * Math.min(1, Math.max(0, now - instant(end)) / WINDOW_MS);
}

/**
 * **A card is the size of a picture, and a picture is the size of its shot.** Shots are
 * rendered at a 16:9 frame and stored 480x270 (`view_shots.rs`), so 240x135 draws one at
 * exactly 2x with nothing cropped. A card is the same box because the two sit in one rank as
 * peers of one another: two sizes there made the rank read as two ranks.
 */
const CORE_W = 340, CORE_H = 320, CARD_W = 240, CARD_H = 135;
/**
 * The gutter between one rank and the next. Wider than it needs to be to keep boxes apart,
 * because it is also the room the wires bend in: a wire leaves its parent's edge, runs to the
 * midpoint and arrives flat at its child, so a narrow gutter makes every curve the same
 * near-vertical kink and the branch that owns a card stops being readable from its wire.
 * It was sized when the chart was fitted and width was nearly free; at 1x it is not, and a
 * two-sided chart is about 1604px wide, so a laptop window pans sideways as well as down.
 */
const GAP_X = 64, MARGIN = 24;
/**
 * The air between two adjacent nodes, by **the rank their branches part at** — not by how deep
 * either of them happens to sit.
 *
 * One gap for every pair drew the ranks as one flat column: a task's own results sat 14px from
 * each other and 14px from the next task's results, so nothing in the spacing said which
 * branch anything belonged to, and the reading had to be done off the wires alone. Divergence
 * is what a rank actually is. Two nodes that part at the core are a core's gap apart wherever
 * they are in the tree; only nodes that share a parent get the tight one, and the tightness
 * then reads as belonging together rather than as crowding.
 *
 * Indexed by rank, so `[0]` is the core's own and is never asked for — the core has no sibling.
 */
const RANK_GAP = [0, 40, 20, 12];
const gapAt = (rank) => RANK_GAP[Math.min(rank, RANK_GAP.length - 1)];
/** The rank two nodes' branches part at: one below their nearest common ancestor's. */
function divergence(a, b) {
  while (a.depth > b.depth) a = a.parent;
  while (b.depth > a.depth) b = b.parent;
  while (a !== b && a.parent && b.parent) { a = a.parent; b = b.parent; }
  return a.depth + 1;
}
function dimensions() {
  return { w: CARD_W, h: CARD_H };
}

/**
 * **Home opens at 1x, and any other scale is the person's.** It used to open fitted to the
 * window with a floor of 0.7, and the floor was where every real day landed: fitting shrank
 * every card to 70% — a 17px title drawn at 12px, a 16:9 picture at 168x95 — to buy the one
 * view of the whole chart that nobody was reading at that size. A day that is wider or taller
 * than the window scrolls from the core outward instead, and pinch or ⌘/Ctrl-wheel is there
 * for the moment the shape of the whole is what someone wants.
 *
 * The range reaches out far enough to see that shape and in far enough to read a picture.
 */
const ZOOM_MIN = 0.25, ZOOM_MAX = 2;
const clampZoom = (scale) => Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, scale));

/** Where the drawing sits at a scale: centred on an axis it is smaller than the window on. */
function stage(chart, frame, scale) {
  const drawn = { w: chart.width * scale, h: chart.height * scale };
  const canvas = { w: Math.max(Math.floor(frame.w), Math.ceil(drawn.w)),
    h: Math.max(Math.floor(frame.h), Math.ceil(drawn.h)) };
  return { canvas, offset: { x: (canvas.w - drawn.w) / 2, y: (canvas.h - drawn.h) / 2 } };
}

/** The scroll that keeps the chart point under `point` (window pixels) there across a zoom. */
function zoomAround(chart, frame, from, to, scroll, point) {
  const before = stage(chart, frame, from).offset, after = stage(chart, frame, to).offset;
  const x = (scroll.left + point.x - before.x) / from, y = (scroll.top + point.y - before.y) / from;
  return { left: after.x + x * to - point.x, top: after.y + y * to - point.y };
}

/**
 * Semantic edges determine the hierarchy; flextree only computes its geometry.
 * Overview children are embedded INSIDE the core, not duplicated as peripheral cards.
 *
 * **The whole tree is drawn, always.** There is no collapse, because with the work in hand
 * and nothing else there is nothing to hide from: the instance that laid out 225 cards over
 * 2556x16529px lays out 20.
 */
function arrange(model) {
  const children = childIndex(model);
  const branches = children.get(model.rootId).filter((n) => n.kind !== "overview");
  const tree = (node) => ({ node, ...dimensions(node),
    children: (children.get(node.id) || []).map(tree) });
  const weight = (t, rank = 1) => Math.max(t.h,
    t.children.reduce((n, c) => n + weight(c, rank + 1) + gapAt(rank + 1), 0));
  const sides = [[], []], load = [0, 0];
  // IDs, not activity states, keep branch placement stable between updates.
  for (const branch of [...branches].sort((a, b) => a.id.localeCompare(b.id))) {
    const t = tree(branch), side = load[0] <= load[1] ? 0 : 1;
    sides[side].push(t); load[side] += weight(t);
  }
  const placed = [{ node: model.nodes[0], x: -CORE_W / 2, y: -CORE_H / 2, w: CORE_W, h: CORE_H, dir: 0 }];
  for (let side = 0; side < 2; side++) {
    if (!sides[side].length) continue;
    // The node's own extent only; the air between two of them is `spacing`, which sees both
    // and so can ask where they parted. A padded `nodeSize` cannot — it is one number per node.
    const layout = flextree({ nodeSize: (n) => [n.data.h, n.data.w + GAP_X],
      spacing: (a, b) => gapAt(divergence(a, b)) });
    const root = layout.hierarchy({ w: CORE_W / 2, h: CORE_H, children: sides[side] });
    layout(root);
    const dir = side === 0 ? 1 : -1;
    for (const n of root.descendants().slice(1)) {
      placed.push({ node: n.data.node, w: n.data.w, h: n.data.h, dir,
        x: dir > 0 ? n.y : -n.y - n.data.w, y: n.x - n.data.h / 2 });
    }
  }
  const x0 = Math.min(...placed.map((n) => n.x)) - MARGIN;
  const y0 = Math.min(...placed.map((n) => n.y)) - MARGIN;
  const width = Math.max(...placed.map((n) => n.x + n.w)) - x0 + MARGIN;
  const height = Math.max(...placed.map((n) => n.y + n.h)) - y0 + MARGIN;
  for (const row of placed) { row.x -= x0; row.y -= y0; }
  const byId = new Map(placed.map((p) => [p.node.id, p]));
  const wires = model.edges.filter((e) => e.primary && byId.has(e.from) && byId.has(e.to)).map((edge) => {
    const from = byId.get(edge.from), to = byId.get(edge.to), right = to.dir > 0;
    const sx = from.x + (right ? from.w : 0), sy = from.y + from.h / 2;
    const ex = to.x + (right ? 0 : to.w), ey = to.y + to.h / 2, mid = (sx + ex) / 2;
    return { ...edge, tone: stateOf(to.node), d: `M ${sx} ${sy} C ${mid} ${sy}, ${mid} ${ey}, ${ex} ${ey}` };
  });
  return { placed, wires, width, height };
}

async function getJson(path) {
  const response = await fetch(path);
  if (!response.ok) throw new Error(`${response.status}`);
  return response.json();
}

export default function Home() {
  const { openRef } = useViews();
  const { messages } = useMessages();
  const [source, setSource] = useState({ tasks: [], workers: [], views: [] });
  const [loaded, setLoaded] = useState(false);
  const [errors, setErrors] = useState([]);
  const [now, setNow] = useState(Date.now);
  const [frame, setFrame] = useState({ w: 1200, h: 760 });
  const viewport = useRef(null), inFlight = useRef(false);
  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      // `/api/workers/ended` is gone from this view: an ended session is not in hand, and
      // `factory/workers` is the surface that owns session history.
      const requests = [
        ["workers", "/api/workers", (v) => v.workers],
        ["views", "/api/views", (v) => v],
      ];
      const results = await Promise.allSettled(requests.map(async ([, path, read]) => {
        const value = read(await getJson(path));
        if (!Array.isArray(value)) throw new Error("Invalid source");
        return value;
      }));
      // Failure retains the last good snapshot AND marks it stale, never an empty success.
      setSource((prev) => {
        const next = { ...prev };
        results.forEach((result, i) => { if (result.status === "fulfilled") next[requests[i][0]] = result.value; });
        return next;
      });
      setErrors(results.flatMap((result, i) => result.status === "rejected" ? [requests[i][0]] : []));
      setNow(Date.now()); setLoaded(true);
    } finally { inFlight.current = false; }
  }, []);
  useLive(refresh, { period: TEMPO.ledger });
  // Durable history is a ledger read, not a 2-second activity scan. **The ledger is not read
  // on a clock at all.** It is the heaviest source and the one that can say when it moved, so
  // this parks on its version: nothing crosses the wire while nothing is written, and a record
  // written on disk is here in a few hundred ms rather than by the next tick.
  const readLedger = useCallback(async (since) => {
    const at = since === null || since === undefined ? "" : `?since=${encodeURIComponent(since)}`;
    let answer;
    try {
      answer = await getJson(`/api/tasks${at}`);
    } catch {
      setErrors((prev) => (prev.includes("tasks") ? prev : [...prev, "tasks"]));
      return null;
    }
    setErrors((prev) => prev.filter((source) => source !== "tasks"));
    if (answer.unchanged) return answer.version;
    if (!Array.isArray(answer.tasks)) return null;
    setSource((prev) => ({ ...prev, tasks: answer.tasks }));
    setNow(Date.now());
    setLoaded(true);
    return answer.version;
  }, []);
  useWatched(readLedger);
  const model = useMemo(() => buildHome({ ...source, messages }, now), [source, messages, now]);
  const children = useMemo(() => childIndex(model), [model]);
  const chart = useMemo(() => arrange(model), [model]);
  const mobile = frame.w < 760;
  useEffect(() => {
    const el = viewport.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => setFrame({ w: entry.contentRect.width, h: entry.contentRect.height }));
    observer.observe(el); return () => observer.disconnect();
  }, []);
  const [scale, setScale] = useState(1);
  const { canvas, offset } = stage(chart, frame, scale);
  // Wheel and gesture events arrive faster than renders, so each zoom composes on the scale and
  // scroll the previous one asked for rather than on what is on screen yet.
  const live = useRef({ scale, chart, frame }), pending = useRef(null);
  live.current = { scale: pending.current?.scale ?? scale, chart, frame };
  const zoomTo = useCallback((next, point) => {
    const el = viewport.current;
    if (!el) return;
    const { scale: from, chart, frame } = live.current;
    const to = clampZoom(next);
    if (to === from) return;
    const scroll = pending.current || { left: el.scrollLeft, top: el.scrollTop };
    const at = point || { x: frame.w / 2, y: frame.h / 2 };
    pending.current = { scale: to, ...zoomAround(chart, frame, from, to, scroll, at) };
    live.current.scale = to;
    setScale(to);
  }, []);
  useLayoutEffect(() => {
    if (!pending.current || !viewport.current) return;
    viewport.current.scrollTo({ left: pending.current.left, top: pending.current.top, behavior: "instant" });
    pending.current = null;
  }, [scale]);
  // Centre the core on first paint — which does nothing while the chart fits. Past that the
  // scroll position is the person's.
  const centred = useRef(false);
  useLayoutEffect(() => {
    if (!loaded || mobile || !viewport.current || centred.current) return;
    centred.current = true;
    const core = chart.placed[0];
    viewport.current.scrollTo({ left: offset.x + (core.x + core.w / 2) * scale - frame.w / 2,
      top: offset.y + (core.y + core.h / 2) * scale - frame.h / 2, behavior: "instant" });
  }, [loaded, mobile, chart, frame, scale, offset.x, offset.y]);
  const pointers = useRef(new Map()), gesture = useRef(null), dragged = useRef(false);
  // Pinch and ⌘/Ctrl-wheel zoom at the pointer. A plain wheel still scrolls. These are
  // registered by hand because React's wheel listener is passive and cannot stop the page
  // itself from zooming; `gesture*` is how Safari and WKWebView report a trackpad pinch.
  useEffect(() => {
    const el = viewport.current;
    if (!el || mobile) return;
    const at = (e) => { const r = el.getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; };
    const wheel = (e) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      const dy = Math.max(-50, Math.min(50, e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY));
      zoomTo(live.current.scale * Math.exp(-dy * 0.005), at(e));
    };
    let base = 1;
    // iOS reports a touch pinch as gesture events AND as pointers; the pointers already zoom it.
    const start = (e) => { e.preventDefault(); base = live.current.scale; };
    const change = (e) => { e.preventDefault(); if (pointers.current.size < 2) zoomTo(base * e.scale, at(e)); };
    el.addEventListener("wheel", wheel, { passive: false });
    el.addEventListener("gesturestart", start);
    el.addEventListener("gesturechange", change);
    return () => {
      el.removeEventListener("wheel", wheel);
      el.removeEventListener("gesturestart", start);
      el.removeEventListener("gesturechange", change);
    };
  }, [mobile, zoomTo]);
  // Drag pans from anywhere, cards included, and two touches pinch. A press only becomes a
  // drag past a few pixels, so a tap on a card still opens it, and the click that ends a real
  // drag is swallowed rather than opening whatever the pointer happened to be over.
  const beginPan = (el, id) => {
    const p = pointers.current.get(id);
    gesture.current = { id, x: p.x, y: p.y, left: el.scrollLeft, top: el.scrollTop };
  };
  // A lifted finger ends a pinch; the one still down carries on as a pan from where it is.
  const release = (e) => {
    pointers.current.delete(e.pointerId);
    const [rest] = pointers.current.keys();
    gesture.current = null;
    if (rest !== undefined) beginPan(e.currentTarget, rest);
  };
  const pinchOf = (el) => {
    const [a, b] = [...pointers.current.values()], r = el.getBoundingClientRect();
    return { dist: Math.hypot(a.x - b.x, a.y - b.y) || 1, at: { x: (a.x + b.x) / 2 - r.left, y: (a.y + b.y) / 2 - r.top } };
  };
  const branches = children.get("core")?.filter((n) => n.kind !== "overview") || [];
  const common = { now, children, openRef };
  return (
    <div className="hi-work">
      <style>{CSS}</style>
      {errors.length > 0 && <div className="hi-work__error" role="status">{L.failed}: {errors.map((key) => L.source[key]).join(" · ")}
        <button onClick={refresh}>{L.retry}</button></div>}
      <div className="hi-work__viewport" ref={viewport} data-chart={mobile ? undefined : ""} onPointerDown={(e) => {
        if (mobile || (e.pointerType === "mouse" && e.button !== 0)) return;
        const el = e.currentTarget;
        pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
        if (pointers.current.size === 1) { dragged.current = false; beginPan(el, e.pointerId); }
        if (pointers.current.size === 2) {
          dragged.current = true;
          for (const id of pointers.current.keys()) el.setPointerCapture(id);
          gesture.current = { pinch: pinchOf(el), scale: live.current.scale };
        }
      }} onPointerMove={(e) => {
        const el = e.currentTarget, g = gesture.current;
        if (!g || !pointers.current.has(e.pointerId)) return;
        pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
        if (g.pinch) {
          const pinch = pinchOf(el);
          zoomTo(g.scale * pinch.dist / g.pinch.dist, pinch.at);
          return;
        }
        const dx = e.clientX - g.x, dy = e.clientY - g.y;
        if (!dragged.current) {
          if (Math.hypot(dx, dy) < 5) return;
          dragged.current = true;
          el.setPointerCapture(e.pointerId);
        }
        el.scrollLeft = g.left - dx;
        el.scrollTop = g.top - dy;
      }} onPointerUp={release} onPointerCancel={release} onClickCapture={(e) => {
        if (!dragged.current) return;
        dragged.current = false;
        e.stopPropagation(); e.preventDefault();
      }}>
        {!loaded && <p className="hi-work__loading" role="status">{L.reading}</p>}
        {loaded && !branches.length && <p className="hi-work__loading" role="status">{L.nothing}</p>}
        {mobile ? <div className="hi-work__flow">
          <Core node={model.nodes[0]} model={model} now={now} />
          <Branch nodes={branches} {...common} />
        </div> : <div className="hi-work__canvas" style={{ width: canvas.w, height: canvas.h }}>
          <div className="hi-work__stage" style={{ left: offset.x, top: offset.y, width: chart.width, height: chart.height,
            transform: `scale(${scale})` }}>
            <svg className="hi-work__wires" width={chart.width} height={chart.height} aria-hidden>
              {chart.wires.map((wire) => <path key={wire.id} data-edge={wire.id} d={wire.d} stroke={TONE[wire.tone] || TONE.todo} />)}
            </svg>
            {chart.placed.map((row) => <div key={row.node.id} className="hi-work__position"
              style={{ left: row.x, top: row.y, width: row.w, height: row.h }}>
              {row.node.kind === "core" ? <Core node={model.nodes[0]} model={model} now={now} />
                : <Node node={row.node} {...common} />}
            </div>)}
          </div>
        </div>}
      </div>
    </div>
  );
}

/** Read-only. Every affordance this card had opened a detail dialog that no longer exists. */
function Core({ node, model, now }) {
  const overview = node.data.overviewIds.map((id) => model.nodes.find((n) => n.id === id)).filter(Boolean);
  const messages = overview.filter((n) => n.sourceRefs.some((r) => r.kind === "message"));
  const updates = overview.filter((n) => !messages.includes(n));
  return <article className="hi-work__core" data-node-id="core">
    <h2 className="hi-work__core-title"><span className="hi-work__pip" />{node.title}</h2>
    <div className="hi-work__roles">
      {[...CORE_ROLES].map((role) => {
        const s = node.data.sessions.find((s) => s.role === role);
        return <span key={role} title={`${L.roles[role]} · ${L.status[s?.state || "missing"]}`}>
          <i className="hi-work__live" data-live={s?.state === "running"} />{L.roles[role]}<small>{L.status[s?.state || "missing"]}</small>
        </span>;
      })}
    </div>
    <div className="hi-work__overview">
      {!messages.length && <p>{L.noMessages}</p>}
      {messages.map((item) => <div key={item.id} data-node-id={item.id}>
        <span>{item.title}<time>{item.data.freshness === "stale" ? L.stale : age(item.data.updatedAt, now)}</time></span>
        <p>{item.summary}</p>
      </div>)}
      {updates.map((item) => <div key={item.id} data-node-id={item.id} className="hi-work__update">
        <span>{L.update}<time>{age(item.data.updatedAt, now)}</time></span>
        <p>{item.data.text}</p>
      </div>)}
    </div>
  </article>;
}

/**
 * A card is a glance, and its whole surface is one handoff.
 *
 * **What a card is, it says without a label.** There are two kinds of card — a task and a live
 * session working on one — and a word naming the kind was the first line of every card. A
 * session instead wears the dot the core's roles wear, because it is the same fact: the dot
 * means a live session, filled while it is running. A task has none. The status word carries
 * the tone a coloured left edge used to, so no side of the border means anything.
 *
 * **Named loan:** the handoff opens the board, not this row on it. `openRef(viewRef)` carries
 * a view ref and nothing else, and `factory/tasks` reads no incoming target, so there is no
 * way to say "open the board focused here" — giving the view-open a target means changing the
 * wire, the server and the view contract together, which is its own piece of work. Until that
 * lands you arrive at the board and find the row yourself. The item that takes this back is
 * a targeted view-open; see `docs/arch/home.md` § Open.
 */
function Node({ node, now, children, openRef }) {
  const state = stateOf(node), time = nodeTime(node);
  const board = node.kind === "task" ? TASK_BOARD : node.kind === "activity" ? SESSION_BOARD : null;
  // A tile is the one handoff that lands exactly where it points: `openRef` takes a view ref
  // natively, so this needs none of the targeting the board handoff is waiting on.
  if (node.kind === "result") return <article className="hi-work__tile" data-node-id={node.id} data-kind="result">
    <button onClick={() => openRef(node.data.viewRef)} title={node.title}>
      <img src={node.data.shot} alt={node.title} loading="lazy" />
    </button>
  </article>;
  const body = <>
    <span className="hi-work__node-title">{node.title}</span>
    <div className="hi-work__node-foot"><span className="hi-work__node-state">
      {node.kind === "activity" && <i className="hi-work__live" data-live={node.data.session.state === "running"} />}
      {L.status[state] || ""}</span>
      {["task", "activity"].includes(node.kind) && <time>{age(time, now)}</time>}</div>
  </>;
  return <article className="hi-work__node" data-node-id={node.id} data-kind={node.kind}
    style={{ "--node-tone": TONE[state] || TONE.todo, opacity: emphasis(node, now) }}>
    {board ? <button className="hi-work__open" onClick={() => openRef(board)}>{body}</button> : body}
  </article>;
}

function Branch({ nodes, ...props }) {
  return <ul className="hi-work__branch">{nodes.map((node) => <li key={node.id}>
    <Node node={node} {...props} />
    {props.children.get(node.id)?.length > 0 && <Branch nodes={props.children.get(node.id)} {...props} />}
  </li>)}</ul>;
}

const CSS = `
.hi-work { --bg: var(--bg-0); --work-line: color-mix(in srgb, var(--fg-mute) 28%, transparent); height:100%; min-height:0; position:relative; display:flex; flex-direction:column; color:var(--fg); background:var(--bg); padding-top:var(--hi-safe-top, 0px); font-family:var(--font-display, sans-serif); letter-spacing:0; }
.hi-work *, .hi-work *::before, .hi-work *::after { box-sizing:border-box; }
.hi-work button { font:inherit; color:inherit; background:none; border:0; padding:0; cursor:pointer; text-align:left; }
.hi-work button:focus-visible { outline:2px solid var(--accent); outline-offset:2px; }
.hi-work__error { padding:8px 24px; color:var(--danger); font-size:13px; display:flex; align-items:center; gap:12px; }
.hi-work__error button { text-decoration:underline; min-height:36px; }
.hi-work__viewport { flex:1; min-height:0; overflow:auto; overscroll-behavior:contain; position:relative; touch-action:pan-x pan-y; padding-bottom:100px; }
.hi-work__viewport[data-chart] { touch-action:none; cursor:grab; }
.hi-work__viewport[data-chart]:active { cursor:grabbing; }
.hi-work__canvas { position:relative; user-select:none; -webkit-user-select:none; }
.hi-work__canvas img { -webkit-user-drag:none; }
.hi-work__stage { position:absolute; transform-origin:0 0; }
.hi-work__wires { position:absolute; left:0; top:0; pointer-events:none; }
.hi-work__wires path { fill:none; stroke-width:1.6; opacity:.7; }
.hi-work__position { position:absolute; }
.hi-work__core { height:100%; display:flex; flex-direction:column; padding:16px 22px; background:var(--bg); border:1px solid var(--work-line); border-radius:6px; }
.hi-work__core-title { display:flex; gap:10px; align-items:center; margin:0; min-height:36px; font-size:24px; font-weight:600; }
.hi-work__pip { width:11px; height:11px; flex:0 0 11px; border-radius:50%; background:var(--accent); }
.hi-work__roles { display:flex; gap:14px; padding:10px 0 12px; border-bottom:1px solid var(--work-line); }
.hi-work__roles span { display:grid; grid-template-columns:7px 1fr; align-items:center; column-gap:5px; font-size:13px; }
.hi-work__live { width:5px; height:5px; flex:0 0 5px; border:1px solid var(--fg-mute); border-radius:50%; }
.hi-work__live[data-live=true] { background:var(--accent); border-color:var(--accent); }
.hi-work__roles small { grid-column:2; font-size:10px; color:var(--fg-mute); }
.hi-work__overview { flex:1; min-height:0; overflow:hidden; padding-top:6px; }
.hi-work__overview > p { font-size:14px; color:var(--fg-mute); }
.hi-work__overview > div { padding:9px 0; }
.hi-work__overview span { display:flex; flex-wrap:wrap; justify-content:space-between; gap:5px; font-size:12px; color:var(--fg-mute); }
.hi-work__overview time { font-size:11px; }
.hi-work__overview p { margin:5px 0 0; font-size:16px; line-height:1.5; overflow:hidden; display:-webkit-box; -webkit-line-clamp:2; -webkit-box-orient:vertical; overflow-wrap:anywhere; }
.hi-work__update p { font-size:14px; color:var(--fg-dim, var(--fg-mute)); }
.hi-work__node { height:100%; background:var(--bg); border:1px solid var(--work-line); border-radius:6px; display:flex; flex-direction:column; }
.hi-work__open, .hi-work__node { padding:10px 14px; }
.hi-work__tile { height:100%; border:1px solid var(--work-line); border-radius:6px; overflow:hidden; background:color-mix(in srgb, var(--fg-mute) 10%, transparent); }
.hi-work__tile button { display:block; width:100%; height:100%; padding:0; }
.hi-work__tile img { display:block; width:100%; height:100%; object-fit:cover; object-position:top center; }
.hi-work__open { display:flex; flex-direction:column; flex:1; min-width:0; height:100%; padding:0; }
.hi-work__node-title { display:-webkit-box; -webkit-line-clamp:2; -webkit-box-orient:vertical; overflow:hidden; font-size:19px; line-height:1.3; overflow-wrap:anywhere; font-weight:500; }
.hi-work__node-foot { display:flex; flex-wrap:wrap; justify-content:space-between; gap:6px; margin-top:auto; padding-top:6px; font-size:13px; color:var(--fg-mute); }
.hi-work__node-foot time { white-space:nowrap; }
.hi-work__node-state { display:flex; align-items:center; gap:6px; color:var(--node-tone); }
.hi-work__flow { max-width:720px; margin:0 auto; padding:20px 16px 80px; }
.hi-work__flow .hi-work__core { height:auto; min-height:320px; }
/* The narrow flow grades its air by rank for the same reason the chart does: nesting alone
   put a task's own results as far from it as the next task's were. Inheriting --rank-gap
   carries the tightest value on down, so a fourth rank is no looser than the third. */
.hi-work__branch { --rank-gap:30px; list-style:none; padding:0 0 0 18px; margin:0 0 0 8px; border-left:1px solid var(--work-line); }
.hi-work__branch .hi-work__branch { --rank-gap:18px; margin-left:0; padding-left:12px; }
.hi-work__branch .hi-work__branch .hi-work__branch { --rank-gap:10px; }
.hi-work__branch li { position:relative; padding-top:var(--rank-gap); min-width:0; }
.hi-work__branch li::before { content:''; position:absolute; width:18px; left:-18px; top:calc(var(--rank-gap) + 32px); border-top:1px solid var(--work-line); }
.hi-work__branch .hi-work__node { min-height:116px; }
.hi-work__branch .hi-work__tile { height:auto; aspect-ratio:16 / 9; max-width:240px; }
.hi-work__branch .hi-work__branch li::before { left:-12px; width:12px; }
.hi-work__loading { position:absolute; left:24px; top:8px; color:var(--fg-mute); font-size:13px; z-index:2; }
@media (max-width:759px) { .hi-work__node-title { min-height:36px; } }
`;
