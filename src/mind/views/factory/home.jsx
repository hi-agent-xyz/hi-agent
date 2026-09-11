// purpose: Home is a semantic work tree rooted in Hi Agent, not a process tree.
// This factory view is compiled as a single JSX module, without bundling local imports.
// Keep the pure read model above Home; home.test.mjs tests it without a React host.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useLive, useMessages, useViews, TEMPO } from "@hi/core";
import { flextree } from "d3-flextree";
import { ChevronDown, ChevronRight, Crosshair, Maximize2, Minus, Plus, Search, X, ExternalLink } from "lucide-react";

const COPY = {
  en: {
    task: "Task", activity: "Activity", topic: "Topic", artifact: "Result", overview: "Overview",
    core: "Hi Agent", context: "Our conversation", update: "Latest update", direction: "Direction",
    decision: "Your decision", nothing: "No recent work", reading: "Loading work...",
    failed: "Some sources could not be refreshed", retry: "Retry", stale: "Earlier context",
    unknown: "Time unknown", untitled: "Untitled activity", search: "Search work and conversations",
    noResults: "No matching work", expand: "Expand", collapse: "Collapse", detail: "Details",
    fit: "Fit work", center: "Back to core", zoomIn: "Zoom in", zoomOut: "Zoom out", close: "Close",
    open: "Open result", related: "Related work", session: "Session", action: "Last observed action",
    lastTurn: "Last turn", history: "Session history", previous: "Previous sessions", noMessages: "No conversation yet",
    status: { todo: "To do", doing: "In progress", serving: "On duty", done: "Completed", cancelled: "Cancelled",
      running: "Working", waiting: "Work queued", idle: "Idle", ended: "Ended", restart: "Interrupted by restart",
      failed: "Last turn failed", interrupted: "Last turn interrupted", completed: "Last turn completed", missing: "Not connected" },
    roles: { reaction: "Conversation", cognition: "Coordination", reflection: "Review" },
    source: { tasks: "tasks", workers: "live sessions", ended: "session history", views: "results" },
    counts: (t, s) => `${t} tasks · ${s} sessions`,
    ago: (n, unit) => `${n}${unit} ago`,
  },
  zh: {
    task: "任务", activity: "活动", topic: "主题", artifact: "成果", overview: "概览",
    core: "Hi Agent", context: "我们的交流", update: "最新更新", direction: "整体方向",
    decision: "需要你决定", nothing: "暂无近期工作", reading: "正在读取工作...",
    failed: "部分数据未能刷新", retry: "重试", stale: "较早的上下文",
    unknown: "时间未知", untitled: "未命名活动", search: "搜索任务与会话",
    noResults: "没有匹配的工作", expand: "展开", collapse: "收起", detail: "详情",
    fit: "适应画布", center: "回到核心", zoomIn: "放大", zoomOut: "缩小", close: "关闭",
    open: "打开成果", related: "相关工作", session: "会话", action: "最近观察到的动作",
    lastTurn: "最近一轮", history: "会话记录", previous: "之前的会话", noMessages: "还没有对话",
    status: { todo: "待开始", doing: "进行中", serving: "值守", done: "已完成", cancelled: "已取消",
      running: "正在处理", waiting: "有工作待处理", idle: "空闲", ended: "已结束", restart: "重启中断",
      failed: "上一轮失败", interrupted: "上一轮中断", completed: "上一轮完成", missing: "未连接" },
    roles: { reaction: "交流", cognition: "协调", reflection: "回顾" },
    source: { tasks: "任务", workers: "在线会话", ended: "历史会话", views: "成果" },
    counts: (t, s) => `${t} 个任务 · ${s} 个会话`,
    ago: (n, unit) => `${n}${{ m: "分钟", h: "小时", d: "天" }[unit]}前`,
  },
};
const L = typeof document !== "undefined" && /^zh/i.test(document.documentElement.lang || navigator.language)
  ? COPY.zh : COPY.en;
const HOURS = 24;
const WINDOW_MS = HOURS * 3600000;
const CORE_ROLES = new Set(["reaction", "cognition", "reflection"]);
const OPEN = new Set(["todo", "doing", "serving"]);
const TONE = { core: "var(--accent)", topic: "var(--fg-mute)", todo: "var(--fg-mute)",
  doing: "var(--accent)", serving: "var(--accent-2)", done: "var(--fg-mute)", cancelled: "var(--fg-mute)",
  running: "var(--accent)", waiting: "var(--accent-2)", idle: "var(--fg-mute)", ended: "var(--fg-mute)",
  restart: "var(--danger)", failed: "var(--danger)", interrupted: "var(--danger)",
  overview: "var(--accent)", artifact: "var(--accent-2)" };

/**
 * HomeModel is a read projection, NEVER another task/session lifecycle store.
 *
 * HomeNode = { id, kind, title, summary?, sourceRefs: SourceRef[], data: NodeData }
 * NodeData is discriminated by kind:
 * - core: { sessions: SessionState[], overviewIds: string[] }
 * - task: { task: TaskDto, status, endedAt, contextOnly }
 * - activity: { session: SessionState, taskId?, ownerSessionId?, currentAction? }
 * - topic: { label }; overview: { category, text, updatedAt, freshness, relatedNodeIds }
 * - artifact: { viewRef?, shot?, url? }
 * HomeEdge = { id, from, to, relation, primary }
 * Every non-root node has ONE primary parent; non-primary edges reference an existing
 * object without duplicating it. Nodes have no x/y, collapsed or selected properties.
 *
 * Internal mappings:
 * 1. TaskDto (/api/tasks) -> task:<subject>. Status/time are copied, not inferred from
 *    sessions. All open tasks + all done/cancelled within 24h, including text-only results.
 * 2. Registry Status (/api/workers) -> session:<run>:<id>. running means busy; waiting
 *    means QUEUED WORK, not waiting for the user; idle is still a LIVE session.
 * 3. Session index Ended (/api/workers/ended?since=...) -> the SAME composite identity.
 *    A finished turn does not end a session. Unknown crash end times are not fabricated.
 * 4. Reaction/Cognition/Reflection sessions belong to the one core (including retained
 *    ended instances); all other sessions become activities, even without a task.
 * 5. subject is the authoritative activity -> task join. owner is a technical session
 *    relationship only; it must not pretend that two independent tasks are one topic.
 * 6. Overview uses useMessages()'s USER-VISIBLE transcript and factual task transitions.
 *    No registry tail, raw reasoning, or tool log is used to manufacture a public plan.
 *    The direction/decision-needed categories are reserved for explicitly published,
 *    source-backed statements; runtime busy flags cannot produce them.
 * 7. Task extra.project / extra.systems provide explicit topic names. Similar prose is
 *    not evidence of ownership. Unfiled nodes connect directly to the core.
 * 8. Results resolve existing task references against /api/views; an image is optional.
 *
 * A view is a single-file transform. Pure model/layout functions stay here rather than
 * importing an unserved local module. The tests evaluate only this section.
 */
const instant = (value) => {
  const n = value ? Date.parse(value) : NaN;
  return Number.isFinite(n) ? n : null;
};
const within = (value, now) => instant(value) === null || instant(value) >= now - WINDOW_MS;
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

function normalizeSession(raw, online) {
  return {
    id: sessionKey(raw), run: raw.run, slug: raw.id || raw.session, role: raw.role,
    title: plain(raw.title) || L.untitled, online,
    state: online ? raw.state : "ended", how: online ? null : raw.how,
    startedAt: raw.started || null, endedAt: online ? null : raw.ended || null,
    stateSince: online ? raw.state_since : raw.ended || null,
    subject: raw.subject || null,
    ownerSessionId: raw.owner ? sessionKey({ run: raw.run, id: raw.owner }) : null,
    // doing survives a turn in the registry; never present stale work as current.
    currentAction: online && raw.state === "running" && raw.doing
      ? { text: raw.doing, at: raw.doing_at || null } : null,
    lastTurn: online ? raw.last_turn || null : null,
  };
}

function taskTopics(task) {
  const field = (key) => (task.extra || []).find((f) => f.key === key && !f.clipped)?.value;
  const names = String(field("project") || field("systems") || "").split(",").map(plain).filter(Boolean);
  return [...new Set(names)];
}

function resolveViews(task, views) {
  const text = [task.body, ...(task.timeline || []).map((m) => m.text)].join("\n");
  // Match explicit view references, not arbitrary title substrings. URL and source-path
  // forms are both written by existing task authors. Only known views can be opened.
  const mentioned = new Set();
  for (const match of text.matchAll(/(?:views\/|view_ref\s*[:=]\s*["'`]?)([\w/-]+)(?:\.jsx)?/g)) {
    mentioned.add(match[1]);
  }
  return views.filter((v) => v.view_ref && (mentioned.has(v.view_ref) ||
    text.includes(`\`${v.view_ref}\``) || text.includes(`"${v.view_ref}"`)));
}

function buildHome({ tasks = [], workers = [], ended = [], views = [], messages = [] }, now = Date.now()) {
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
  // Live wins a poll-boundary race against a retained ended record of the same identity.
  const sessions = new Map();
  for (const s of ended) if (within(s.ended, now)) sessions.set(sessionKey(s), normalizeSession(s, false));
  for (const s of workers) sessions.set(sessionKey(s), normalizeSession(s, true));
  const coreSessions = [...sessions.values()].filter((s) => CORE_ROLES.has(s.role));
  const activities = [...sessions.values()].filter((s) => !CORE_ROLES.has(s.role));
  const core = add({ id: "core", kind: "core", title: L.core,
    sourceRefs: coreSessions.map((s) => ref("session", s.id)), data: { sessions: coreSessions, overviewIds: [] } });
  // Retain expired task context while a displayed session still refers to it. Do not
  // relabel that task as open; contextOnly records WHY it is present beyond the window.
  const subjects = new Set(activities.map((s) => s.subject).filter(Boolean));
  for (const task of tasks) {
    const retained = OPEN.has(task.status) || within(taskEnd(task), now);
    if (!retained && !subjects.has(task.subject)) continue;
    const node = add({ id: taskKey(task.subject), kind: "task", title: task.title || task.subject,
      sourceRefs: [ref("task", task.subject)], data: { task, status: task.status,
        endedAt: taskEnd(task), contextOnly: !retained } });
    const topics = taskTopics(task);
    // Multiple explicit tags are references; the first declared tag is the stable parent.
    for (const [i, name] of topics.entries()) {
      const id = `topic:${name.toLocaleLowerCase()}`;
      if (!byId.has(id)) {
        add({ id, kind: "topic", title: name, sourceRefs: [ref("task", task.subject)], data: { label: name } });
        link("core", id);
      }
      link(id, node.id, "contains", i === 0);
    }
    if (!topics.length) link("core", node.id);
    for (const view of resolveViews(task, views)) {
      const id = `artifact:view:${view.view_ref}`;
      const exists = byId.has(id);
      add({ id, kind: "artifact", title: view.label || view.view_ref,
        sourceRefs: [ref("view", view.view_ref)], data: { viewRef: view.view_ref, shot: view.shot_url || null } });
      link(node.id, id, "produces", !exists);
    }
    for (const file of task.files || []) {
      const id = `artifact:file:${task.subject}/${file.path}`;
      const url = `/api/tasks/${encodeURIComponent(task.subject)}/files/${file.path.split("/").map(encodeURIComponent).join("/")}`;
      add({ id, kind: "artifact", title: file.path, sourceRefs: [ref("file", `${task.subject}/${file.path}`)], data: { url } });
      link(node.id, id, "produces");
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
  for (const node of [...nodes]) {
    if (node.kind !== "task" || OPEN.has(node.data.status) || node.data.contextOnly) continue;
    const id = `overview:task:${node.id}`;
    add({ id, kind: "overview", title: node.title,
      summary: L.status[node.data.status], sourceRefs: node.sourceRefs,
      data: { category: "update", text: `${node.title} · ${L.status[node.data.status]}`,
        updatedAt: node.data.endedAt, freshness: "current", relatedNodeIds: [node.id] } });
    link("core", id, "explains"); link(id, node.id, "explains", false);
    core.data.overviewIds.push(id);
  }
  return { rootId: "core", asOf: new Date(now).toISOString(), nodes, edges,
    counts: { tasks: nodes.filter((n) => n.kind === "task").length, sessions: sessions.size } };
}

function childIndex(model) {
  const children = new Map(model.nodes.map((n) => [n.id, []]));
  const nodes = new Map(model.nodes.map((n) => [n.id, n]));
  for (const edge of model.edges) if (edge.primary && nodes.has(edge.to)) children.get(edge.from)?.push(nodes.get(edge.to));
  return children;
}

function revealPath(model, id, collapsed) {
  const parent = new Map(model.edges.filter((e) => e.primary).map((e) => [e.to, e.from]));
  const next = new Set(collapsed);
  const visited = new Set();
  while (id && !visited.has(id)) {
    visited.add(id); next.delete(id); id = parent.get(id);
  }
  return next;
}

function stateOf(node) {
  if (node.kind === "task") return node.data.status;
  if (node.kind !== "activity") return node.kind;
  const s = node.data.session;
  if (!s.online) return s.how === "restart" ? "restart" : "ended";
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
  const end = node.kind === "task" ? node.data.endedAt
    : node.kind === "activity" && !node.data.session.online ? node.data.session.endedAt : null;
  if (instant(end) === null) return 1;
  // Fade emphasis, not legibility: even at 24h the card and its wire remain readable.
  return 1 - 0.22 * Math.min(1, Math.max(0, now - instant(end)) / WINDOW_MS);
}

const CORE_W = 360, CORE_H = 350, NODE_W = 280, NODE_H = 134;
function dimensions(node) {
  if (node.kind === "topic") return { w: 220, h: 78 };
  if (node.kind === "artifact" && node.data.shot) return { w: NODE_W, h: 252 };
  return { w: NODE_W, h: NODE_H };
}

/** Semantic edges determine the hierarchy; flextree only computes its geometry.
 * Overview children are embedded INSIDE the core, not duplicated as peripheral cards.
 * Recursive node sizes support arbitrary depth; collapsed state never modifies the model.
 */
function arrange(model, collapsed = new Set()) {
  const children = childIndex(model);
  const branches = children.get(model.rootId).filter((n) => n.kind !== "overview");
  const tree = (node) => ({ node, ...dimensions(node),
    children: collapsed.has(node.id) ? [] : (children.get(node.id) || []).map(tree) });
  const weight = (t) => Math.max(t.h, t.children.reduce((n, c) => n + weight(c) + 24, 0));
  const sides = [[], []], load = [0, 0];
  // IDs, not activity states, keep branch placement stable between updates.
  for (const branch of [...branches].sort((a, b) => a.id.localeCompare(b.id))) {
    const t = tree(branch), side = load[0] <= load[1] ? 0 : 1;
    sides[side].push(t); load[side] += weight(t);
  }
  const placed = [{ node: model.nodes[0], x: -CORE_W / 2, y: -CORE_H / 2, w: CORE_W, h: CORE_H, dir: 0 }];
  for (let side = 0; side < 2; side++) {
    if (!sides[side].length) continue;
    const layout = flextree({ nodeSize: (n) => [n.data.h + 24, n.data.w + 90], spacing: 0 });
    const root = layout.hierarchy({ w: CORE_W / 2, h: CORE_H, children: sides[side] });
    layout(root);
    const dir = side === 0 ? 1 : -1;
    for (const n of root.descendants().slice(1)) {
      placed.push({ node: n.data.node, w: n.data.w, h: n.data.h, dir,
        x: dir > 0 ? n.y : -n.y - n.data.w, y: n.x - n.data.h / 2 });
    }
  }
  const x0 = Math.min(...placed.map((n) => n.x)) - 48;
  const y0 = Math.min(...placed.map((n) => n.y)) - 48;
  const width = Math.max(...placed.map((n) => n.x + n.w)) - x0 + 48;
  const height = Math.max(...placed.map((n) => n.y + n.h)) - y0 + 48;
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
  const [source, setSource] = useState({ tasks: [], workers: [], ended: [], views: [] });
  const [loaded, setLoaded] = useState(false);
  const [errors, setErrors] = useState([]);
  const [now, setNow] = useState(Date.now);
  const [collapsed, setCollapsed] = useState(() => new Set());
  const [selected, setSelected] = useState(null);
  const [query, setQuery] = useState("");
  const [scale, setScale] = useState(1);
  const [frame, setFrame] = useState({ w: 1200, h: 760 });
  const viewport = useRef(null), drag = useRef(null), initialized = useRef(false), inFlight = useRef(false);
  const [focusId, setFocusId] = useState("core");
  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const since = encodeURIComponent(new Date(Date.now() - WINDOW_MS).toISOString());
      const requests = [
        ["tasks", "/api/tasks", (v) => v.tasks],
        ["workers", "/api/workers", (v) => v.workers],
        ["ended", `/api/workers/ended?since=${since}`, (v) => {
          if (v.complete !== true) throw new Error("Incomplete session history");
          return v.ended;
        }],
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
  // Durable history is a ledger read, not a 2-second activity scan.
  useLive(refresh, { period: TEMPO.ledger });
  const model = useMemo(() => buildHome({ ...source, messages }, now), [source, messages, now]);
  const children = useMemo(() => childIndex(model), [model]);
  const chart = useMemo(() => arrange(model, collapsed), [model, collapsed]);
  const mobile = frame.w < 760;
  useEffect(() => {
    const el = viewport.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => setFrame({ w: entry.contentRect.width, h: entry.contentRect.height }));
    observer.observe(el); return () => observer.disconnect();
  }, []);
  const fit = useCallback(() => {
    setScale(Math.max(0.85, Math.min(1, frame.w / chart.width, frame.h / chart.height)));
    setFocusId("core");
  }, [frame, chart.width, chart.height]);
  useEffect(() => { if (loaded && !initialized.current) { initialized.current = true; fit(); } }, [loaded, fit]);
  useEffect(() => {
    if (!focusId || mobile) return;
    const row = chart.placed.find((p) => p.node.id === focusId);
    if (!row) return;
    viewport.current?.scrollTo({ left: (row.x + row.w / 2) * scale - frame.w / 2,
      top: (row.y + row.h / 2) * scale - frame.h / 2, behavior: "instant" });
    setFocusId(null);
  }, [focusId, scale, chart, frame, mobile]);
  const toggle = (id) => {
    setCollapsed((previous) => { const next = new Set(previous); next.has(id) ? next.delete(id) : next.add(id); return next; });
    setFocusId(id);
  };
  const focus = (id) => {
    const target = model.nodes.find((n) => n.id === id);
    setCollapsed((prev) => revealPath(model, id, prev));
    setFocusId(target?.kind === "overview" ? "core" : id);
    setSelected(id); setQuery("");
  };
  const matches = query.trim() ? model.nodes.filter((n) =>
    `${n.title} ${n.summary || ""} ${n.kind === "activity" ? n.data.session.slug : ""}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())) : [];
  const selectedNode = model.nodes.find((n) => n.id === selected);
  const core = model.nodes[0];
  const common = { now, children, collapsed, toggle, select: setSelected, openRef };
  return (
    <div className="hi-work">
      <style>{CSS}</style>
      <header className="hi-work__toolbar">
        <span className="hi-work__counts">{L.counts(model.counts.tasks, model.counts.sessions)}</span>
        <div className="hi-work__search">
          <Search size={17} aria-hidden />
          <input aria-label={L.search} placeholder={L.search} value={query} onChange={(e) => setQuery(e.target.value)} />
          {query && <button aria-label={L.close} title={L.close} onClick={() => setQuery("")}><X size={16} /></button>}
          {query && <div className="hi-work__results" role="region" aria-label={L.search}>
            {!matches.length && <p>{L.noResults}</p>}
            {matches.map((node) => <button key={node.id} onClick={() => focus(node.id)}>
              <span>{node.title}</span><small>{L[node.kind] || node.kind}</small>
            </button>)}
          </div>}
        </div>
        {!mobile && <div className="hi-work__tools">
          <button title={L.zoomOut} aria-label={L.zoomOut} disabled={scale <= 0.5} onClick={() => setScale((s) => Math.max(0.5, s - 0.1))}><Minus size={18} /></button>
          <output>{Math.round(scale * 100)}%</output>
          <button title={L.zoomIn} aria-label={L.zoomIn} disabled={scale >= 1.5} onClick={() => setScale((s) => Math.min(1.5, s + 0.1))}><Plus size={18} /></button>
          <button title={L.fit} aria-label={L.fit} onClick={fit}><Maximize2 size={18} /></button>
          <button title={L.center} aria-label={L.center} onClick={() => { setFocusId("core"); viewport.current?.scrollTo({
            left: (chart.placed[0].x + CORE_W / 2) * scale - frame.w / 2,
            top: (chart.placed[0].y + CORE_H / 2) * scale - frame.h / 2 }); }}><Crosshair size={18} /></button>
        </div>}
      </header>
      {errors.length > 0 && <div className="hi-work__error" role="status">{L.failed}: {errors.map((key) => L.source[key]).join(" · ")}
        <button onClick={refresh}>{L.retry}</button></div>}
      <div className="hi-work__viewport" ref={viewport} onPointerDown={(e) => {
        if (mobile || e.button !== 0 || e.target.closest("button,input,a,article")) return;
        drag.current = { x: e.clientX, y: e.clientY, left: e.currentTarget.scrollLeft, top: e.currentTarget.scrollTop };
        e.currentTarget.setPointerCapture(e.pointerId);
      }} onPointerMove={(e) => {
        if (!drag.current) return;
        e.currentTarget.scrollLeft = drag.current.left - e.clientX + drag.current.x;
        e.currentTarget.scrollTop = drag.current.top - e.clientY + drag.current.y;
      }} onPointerUp={() => { drag.current = null; }} onPointerCancel={() => { drag.current = null; }}>
        {!loaded && <p className="hi-work__loading" role="status">{L.reading}</p>}
        {mobile ? <div className="hi-work__flow">
          <Core node={core} model={model} select={setSelected} now={now} />
          <Branch nodes={children.get("core").filter((n) => n.kind !== "overview")} {...common} />
        </div> : <div className="hi-work__extent" style={{ width: chart.width * scale, height: chart.height * scale }}>
          <div className="hi-work__canvas" style={{ width: chart.width, height: chart.height, transform: `scale(${scale})` }}>
            <svg className="hi-work__wires" width={chart.width} height={chart.height} aria-hidden>
              {chart.wires.map((wire) => <path key={wire.id} data-edge={wire.id} d={wire.d} stroke={TONE[wire.tone] || TONE.todo} />)}
            </svg>
            {chart.placed.map((row) => <div key={row.node.id} className="hi-work__position"
              style={{ left: row.x, top: row.y, width: row.w, height: row.h }}>
              {row.node.kind === "core" ? <Core node={core} model={model} select={setSelected} now={now} />
                : <Node node={row.node} {...common} />}
            </div>)}
          </div>
        </div>}
      </div>
      {selectedNode && <Details node={selectedNode} model={model} now={now} close={() => setSelected(null)} focus={focus} openRef={openRef} />}
    </div>
  );
}

function Core({ node, model, select, now }) {
  const overview = node.data.overviewIds.map((id) => model.nodes.find((n) => n.id === id));
  const messages = overview.filter((n) => n.sourceRefs.some((r) => r.kind === "message"));
  const updates = overview.filter((n) => !messages.includes(n));
  return <article className="hi-work__core" data-node-id="core">
    <button className="hi-work__core-title" onClick={() => select("core")}><span className="hi-work__pip" />{node.title}</button>
    <div className="hi-work__roles">
      {[...CORE_ROLES].map((role) => {
        const s = node.data.sessions.find((s) => s.role === role && s.online);
        return <button key={role} onClick={() => select("core")} title={`${L.roles[role]} · ${L.status[s?.state || "missing"]}`}>
          <i data-live={s?.state === "running"} />{L.roles[role]}<small>{L.status[s?.state || "missing"]}</small>
        </button>;
      })}
    </div>
    <div className="hi-work__overview">
      {!messages.length && <p>{L.noMessages}</p>}
      {messages.map((item) => <button key={item.id} data-node-id={item.id} onClick={() => select(item.id)}>
        <span>{item.title}<time>{item.data.freshness === "stale" ? L.stale : age(item.data.updatedAt, now)}</time></span>
        <p>{item.summary}</p>
      </button>)}
    </div>
    <button className="hi-work__core-more" onClick={() => select("core")}>
      <span>{updates.length ? `${L.update} · ${updates.length}` : L.detail}</span><ChevronRight size={16} />
    </button>
  </article>;
}

function Node({ node, now, children, collapsed, toggle, select, openRef }) {
  const count = children.get(node.id)?.length || 0;
  const state = stateOf(node), time = nodeTime(node);
  return <article className="hi-work__node" data-node-id={node.id} data-kind={node.kind}
    style={{ "--node-tone": TONE[state] || TONE.todo, opacity: emphasis(node, now) }}>
    {node.kind === "artifact" && node.data.shot && <button className="hi-work__image" onClick={() => openRef(node.data.viewRef)} title={L.open}>
      <img src={node.data.shot} alt={node.title} loading="lazy" />
    </button>}
    <div className="hi-work__node-head"><span>{L[node.kind]}</span>
      {count > 0 && <button title={`${collapsed.has(node.id) ? L.expand : L.collapse} · ${count}`}
        aria-label={`${collapsed.has(node.id) ? L.expand : L.collapse} ${node.title}`} aria-expanded={!collapsed.has(node.id)} onClick={() => toggle(node.id)}>
        <small>{count}</small>{collapsed.has(node.id) ? <ChevronRight size={16} /> : <ChevronDown size={16} />}
      </button>}
    </div>
    <button className="hi-work__node-title" onClick={() => select(node.id)} title={node.title}>{node.title}</button>
    <div className="hi-work__node-foot"><span>{L.status[state] || ""}</span>{["task", "activity"].includes(node.kind) && <time>{age(time, now)}</time>}</div>
  </article>;
}

function Branch({ nodes, ...props }) {
  return <ul className="hi-work__branch">{nodes.map((node) => <li key={node.id}>
    <Node node={node} {...props} />
    {!props.collapsed.has(node.id) && props.children.get(node.id)?.length > 0 &&
      <Branch nodes={props.children.get(node.id)} {...props} />}
  </li>)}</ul>;
}

function Details({ node, model, now, close, focus, openRef }) {
  const dialog = useRef(null);
  useEffect(() => {
    const previous = document.activeElement;
    dialog.current?.showModal();
    return () => { if (previous instanceof HTMLElement) previous.focus(); };
  }, []);
  const children = childIndex(model).get(node.id) || [];
  const related = model.edges.filter((e) => !e.primary && (e.from === node.id || e.to === node.id))
    .map((e) => model.nodes.find((n) => n.id === (e.from === node.id ? e.to : e.from))).filter(Boolean);
  const session = node.kind === "activity" ? node.data.session : null;
  const sessions = node.kind === "core" ? node.data.sessions : session ? [session] : [];
  return <dialog ref={dialog} className="hi-work__dialog" onCancel={close} onClick={(e) => { if (e.target === e.currentTarget) close(); }}>
    <div className="hi-work__detail-body">
      <header><div><small>{L[node.kind] || L.detail}</small><h2>{node.title}</h2></div>
        <button autoFocus aria-label={L.close} title={L.close} onClick={close}><X size={20} /></button></header>
      {node.kind === "overview" && <><time>{age(node.data.updatedAt, now)}{node.data.freshness === "stale" ? ` · ${L.stale}` : ""}</time>
        <p className="hi-work__prose">{node.data.text}</p></>}
      {node.kind === "task" && <><p>{L.status[node.data.status]} · {age(nodeTime(node), now)}</p>
        <p className="hi-work__prose">{node.data.task.body}</p>
        {(node.data.task.timeline || []).map((entry, i) => <div key={i} className="hi-work__moment"><time>{age(entry.at, now)}</time><p>{entry.text}</p></div>)}
      </>}
      {node.kind === "artifact" && <>
        {node.data.shot && <img className="hi-work__detail-image" src={node.data.shot} alt={node.title} />}
        {node.data.viewRef ? <button className="hi-work__command" onClick={() => { close(); openRef(node.data.viewRef); }}><ExternalLink size={16} />{L.open}</button>
          : <a className="hi-work__command" href={node.data.url} target="_blank" rel="noreferrer"><ExternalLink size={16} />{L.open}</a>}
      </>}
      {sessions.map((s) => <section className="hi-work__session" key={s.id}>
        <h3>{L.roles[s.role] || s.title}</h3>
        <p>{L.status[s.online ? s.state : s.how === "restart" ? "restart" : "ended"]} · {age(s.stateSince, now)}</p>
        {s.currentAction && <><h4>{L.action}</h4><pre>{s.currentAction.text}</pre></>}
        {s.lastTurn && <p>{L.status[s.lastTurn.outcome]} · {age(s.lastTurn.at, now)}{s.lastTurn.error && ` · ${s.lastTurn.error}`}</p>}
        <small>{s.run} / {s.slug}</small>
      </section>)}
      {[...children, ...related].length > 0 && <section className="hi-work__related"><h3>{L.related}</h3>
        {[...new Map([...children, ...related].map((n) => [n.id, n])).values()].map((n) =>
          <button key={n.id} onClick={() => focus(n.id)}><span>{n.title}</span><ChevronRight size={16} /></button>)}
      </section>}
    </div>
  </dialog>;
}

// No backticks inside this stylesheet: the factory compiler validates that invariant.
const CSS = `
.hi-work { --bg: var(--bg-0); --work-line: color-mix(in srgb, var(--fg-mute) 28%, transparent); height:100%; min-height:0; display:flex; flex-direction:column; color:var(--fg); background:var(--bg); padding-top:var(--hi-safe-top, 0px); font-family:var(--font-display, sans-serif); letter-spacing:0; }
.hi-work *, .hi-work__dialog * { box-sizing:border-box; }
.hi-work button, .hi-work input, .hi-work__dialog button { font:inherit; color:inherit; }
.hi-work button, .hi-work__dialog button { cursor:pointer; background:none; border:0; }
.hi-work button:focus-visible, .hi-work input:focus-visible, .hi-work__dialog button:focus-visible, .hi-work__dialog a:focus-visible { outline:2px solid var(--accent); outline-offset:2px; }
.hi-work button:disabled { opacity:.35; cursor:default; }
.hi-work__toolbar { display:flex; align-items:center; gap:20px; min-height:64px; padding:8px 24px; border-bottom:1px solid var(--work-line); flex-wrap:wrap; }
.hi-work__counts { font-size:13px; color:var(--fg-mute); white-space:nowrap; }
.hi-work__search { position:relative; display:flex; align-items:center; gap:8px; margin-left:auto; width:300px; max-width:100%; }
.hi-work__search input { width:100%; min-width:0; background:none; border:0; padding:10px 0; font-size:14px; }
.hi-work__search > button { flex:0 0 36px; height:36px; }
.hi-work__results { position:absolute; top:100%; left:0; width:100%; z-index:5; max-height:320px; overflow:auto; background:var(--bg); border:1px solid var(--work-line); box-shadow:0 8px 22px #0002; }
.hi-work__results button { display:flex; width:100%; gap:12px; padding:12px; text-align:left; justify-content:space-between; min-height:44px; border-bottom:1px solid var(--work-line); }
.hi-work__results span { overflow-wrap:anywhere; min-width:0; }
.hi-work__results small { flex-shrink:0; color:var(--fg-mute); }
.hi-work__results p { padding:12px; }
.hi-work__tools { display:flex; align-items:center; gap:2px; }
.hi-work__tools button { width:38px; height:40px; display:grid; place-items:center; }
.hi-work__tools output { font-size:12px; width:44px; text-align:center; }
.hi-work__error { padding:8px 24px; color:var(--danger); font-size:13px; display:flex; align-items:center; gap:12px; }
.hi-work__error button { text-decoration:underline; min-height:36px; }
.hi-work__viewport { flex:1; min-height:0; overflow:auto; overscroll-behavior:contain; position:relative; touch-action:pan-x pan-y; padding-bottom:100px; }
.hi-work__extent { position:relative; min-width:100%; min-height:100%; }
.hi-work__canvas { position:relative; transform-origin:0 0; }
.hi-work__position { position:absolute; }
.hi-work__wires { position:absolute; inset:0; pointer-events:none; }
.hi-work__wires path { fill:none; stroke-width:1.6; opacity:.7; }
.hi-work__core { height:100%; display:flex; flex-direction:column; padding:16px 22px; background:var(--bg); border-top:3px solid var(--accent); border-bottom:1px solid var(--work-line); }
.hi-work__core-title { display:flex; gap:10px; align-items:center; padding:0; min-height:36px; font-size:24px !important; font-weight:600 !important; }
.hi-work__pip { width:11px; height:11px; flex:0 0 11px; border-radius:50%; background:var(--accent); }
.hi-work__roles { display:flex; gap:14px; padding:10px 0 12px; border-bottom:1px solid var(--work-line); }
.hi-work__roles button { display:grid; grid-template-columns:7px 1fr; align-items:center; column-gap:5px; padding:0; text-align:left; font-size:13px; min-height:36px; }
.hi-work__roles i { width:5px; height:5px; border:1px solid var(--fg-mute); border-radius:50%; }
.hi-work__roles i[data-live=true] { background:var(--accent-2); border-color:var(--accent-2); }
.hi-work__roles small { grid-column:2; font-size:10px; color:var(--fg-mute); }
.hi-work__overview { flex:1; min-height:0; overflow:hidden; padding-top:6px; }
.hi-work__overview > p { font-size:14px; color:var(--fg-mute); }
.hi-work__overview button { width:100%; display:block; text-align:left; padding:9px 0; }
.hi-work__overview button > span { display:flex; flex-wrap:wrap; justify-content:space-between; gap:5px; font-size:12px; color:var(--fg-mute); }
.hi-work__overview time { font-size:11px; }
.hi-work__overview p { margin:5px 0 0; font-size:16px; line-height:1.5; overflow:hidden; display:-webkit-box; -webkit-line-clamp:2; -webkit-box-orient:vertical; overflow-wrap:anywhere; }
.hi-work__core-more { display:flex; align-items:center; justify-content:space-between; padding:8px 0 0; border-top:1px solid var(--work-line) !important; font-size:12px !important; min-height:30px; }
.hi-work__node { height:100%; background:var(--bg); border:1px solid var(--work-line); border-left:3px solid var(--node-tone); border-radius:6px; padding:10px 14px; display:flex; flex-direction:column; }
.hi-work__node[data-kind=topic] { border:0; border-bottom:1px solid var(--work-line); border-radius:0; padding-left:0; }
.hi-work__node-head { display:flex; align-items:center; justify-content:space-between; min-height:25px; font-size:11px; color:var(--fg-mute); }
.hi-work__node-head button { display:flex; align-items:center; gap:6px; min-width:40px; min-height:26px; justify-content:flex-end; padding:0; }
.hi-work__node-title { display:-webkit-box; -webkit-line-clamp:2; -webkit-box-orient:vertical; overflow:hidden; text-align:left; padding:0; font-size:18px !important; line-height:1.35 !important; overflow-wrap:anywhere; font-weight:500 !important; }
.hi-work__node-foot { display:flex; flex-wrap:wrap; justify-content:space-between; gap:6px; margin-top:auto; padding-top:8px; font-size:11px; color:var(--fg-mute); }
.hi-work__node-foot > span { color:var(--node-tone); }
.hi-work__image { display:block; padding:0 !important; width:100%; height:142px; flex:0 0 142px; margin-bottom:6px; }
.hi-work__image img { display:block; width:100%; height:100%; object-fit:contain; }
.hi-work__flow { max-width:720px; margin:0 auto; padding:20px 16px 80px; }
.hi-work__flow .hi-work__core { height:auto; min-height:320px; }
.hi-work__branch { list-style:none; padding:0 0 0 18px; margin:0 0 0 8px; border-left:1px solid var(--work-line); }
.hi-work__branch li { position:relative; padding-top:18px; min-width:0; }
.hi-work__branch li::before { content:''; position:absolute; width:18px; left:-18px; top:50px; border-top:1px solid var(--work-line); }
.hi-work__branch .hi-work__node { min-height:116px; }
.hi-work__branch .hi-work__branch { margin-left:0; padding-left:12px; }
.hi-work__branch .hi-work__branch li::before { left:-12px; width:12px; }
.hi-work__loading { position:absolute; left:24px; top:8px; color:var(--fg-mute); font-size:13px; z-index:2; }
.hi-work__dialog { color:var(--fg); background:var(--bg); border:1px solid color-mix(in srgb, var(--fg-mute) 30%, transparent); border-radius:8px; width:600px; max-width:calc(100vw - 32px); max-height:80vh; padding:0; overflow:auto; font-family:var(--font-display, sans-serif); }
.hi-work__dialog::backdrop { background:#0006; }
.hi-work__detail-body { padding:24px; }
.hi-work__detail-body > header { display:flex; justify-content:space-between; align-items:flex-start; gap:16px; }
.hi-work__detail-body header button { min-width:40px; height:40px; display:grid; place-items:center; }
.hi-work__detail-body h2 { margin:6px 0 16px; font-size:22px; line-height:1.35; overflow-wrap:anywhere; }
.hi-work__detail-body h3 { font-size:16px; margin:16px 0 8px; }
.hi-work__detail-body h4 { font-size:13px; font-weight:500; }
.hi-work__detail-body small, .hi-work__detail-body time { font-size:12px; color:var(--fg-mute); overflow-wrap:anywhere; }
.hi-work__prose, .hi-work__moment p { white-space:pre-wrap; overflow-wrap:anywhere; font-size:14px; line-height:1.65; }
.hi-work__moment, .hi-work__session { border-top:1px solid color-mix(in srgb, var(--fg-mute) 25%, transparent); padding-top:12px; margin-top:14px; }
.hi-work__session p { font-size:13px; overflow-wrap:anywhere; }
.hi-work__session pre { white-space:pre-wrap; overflow-wrap:anywhere; font-size:12px; max-height:200px; overflow:auto; }
.hi-work__detail-image { display:block; width:100%; max-height:350px; object-fit:contain; }
.hi-work__command { display:inline-flex; align-items:center; gap:8px; min-height:44px; color:var(--accent); }
.hi-work__related button { display:flex; align-items:center; justify-content:space-between; gap:12px; width:100%; min-height:44px; text-align:left; padding:10px 0; border-bottom:1px solid color-mix(in srgb, var(--fg-mute) 20%, transparent); font-size:14px; }
.hi-work__related span { overflow-wrap:anywhere; }
.hi-work__related svg { flex-shrink:0; }
@media (max-width:759px) { .hi-work__toolbar { padding:8px 16px; gap:4px; } .hi-work__search { width:100%; margin:0; } .hi-work__counts { font-size:12px; } .hi-work__node-head button { min-height:36px; min-width:44px; } .hi-work__node-title { min-height:36px; } }
`;
