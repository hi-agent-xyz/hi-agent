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
      needsYou: "Needs you", running: "Working", waiting: "Work queued", idle: "Idle",
      failed: "Last turn failed", interrupted: "Last turn interrupted", missing: "Not connected" },
    roles: { reaction: "Conversation", cognition: "Coordination", reflection: "Review" },
    source: { tasks: "tasks", workers: "live sessions", views: "results", groups: "grouping" },
    trail: "Where this branch sits", back: "Step back out",
    upkeep: "Upkeep", upkeepNote: "The agent keeping its own house: nobody asked for this work, so no task holds it",
    ago: (n, unit) => `${n}${unit} ago`,
  },
  zh: {
    core: "Hi Agent", context: "我们的交流", update: "最新更新",
    nothing: "手头没有在办的事", reading: "正在读取工作...",
    failed: "部分数据未能刷新", retry: "重试", stale: "较早的上下文",
    unknown: "时间未知", untitled: "未命名活动", noMessages: "还没有对话",
    status: { todo: "待开始", doing: "进行中", serving: "值守", done: "已完成", cancelled: "已取消",
      needsYou: "等你处理", running: "正在处理", waiting: "有工作待处理", idle: "空闲",
      failed: "上一轮失败", interrupted: "上一轮中断", missing: "未连接" },
    roles: { reaction: "交流", cognition: "协调", reflection: "回顾" },
    source: { tasks: "任务", workers: "在线会话", views: "成果", groups: "分组" },
    trail: "这一支所在的位置", back: "退回上一层",
    upkeep: "自身维护", upkeepNote: "Hi Agent 在打理自己：没有人要过这些活，所以没有任务行承载它们",
    ago: (n, unit) => `${n}${{ m: "分钟", h: "小时", d: "天" }[unit]}前`,
  },
};
const L = typeof document !== "undefined" && /^zh/i.test(document.documentElement.lang || navigator.language)
  ? COPY.zh : COPY.en;
const WINDOW_MS = 24 * 3600000;
const CORE_ROLES = new Set(["reaction", "cognition"]);
/**
 * **The agent's own upkeep is a group of its own, and code draws it.** Reflection, a sweep of the
 * ledger, a read of a person's record, a tidy of the workshop: none of that is anything a person
 * asked for, so the ledger holds no row for it (`docs/arch/data.md#tasks`) and dispatch refuses
 * it a `subject` (`hi_create_worker`). That refusal is what makes this a fact rather than a guess:
 * a live session with no subject is upkeep by construction, and Reflection is upkeep by role.
 *
 * It used to hang off the core, beside the person's work, and Reflection sat in the core's role
 * strip. With nothing drawing it, a grouping mind asked to place "the rest" coined a group for the
 * agent's own faults and then filed a person's project under it because the person had said
 * "our code". The group is not in the arrangement record and no mind places anything in it.
 */
const UPKEEP = "upkeep";
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
/** The status word's tone on a card. Only the word: no wire and no border carries status. */
const TONE = { todo: "var(--fg-mute)",
  doing: "var(--accent)", serving: "var(--accent-2)", done: "var(--fg-mute)", cancelled: "var(--fg-mute)",
  needsYou: "var(--danger)",
  running: "var(--accent)", waiting: "var(--accent-2)", idle: "var(--fg-mute)",
  failed: "var(--danger)", interrupted: "var(--danger)" };

/**
 * **A wire's colour says which category it is in, and nothing else.** It used to be the
 * status of the node it pointed at — accent for in progress, accent-2 for on duty and for a
 * picture, grey for the rest — which put a second copy of the status word on every wire, gave
 * accent-2 two unrelated meanings, and at 70% opacity drew "on duty" and "to do" as nearly
 * the same grey. None of it said what the wires are for: which branch a card belongs to.
 *
 * - **A group on the core takes one of eight hues**, evenly spaced around OKLCH at one
 *   lightness and one chroma, so any set of them sits together. The label picks its slot by
 *   hash and a taken slot moves on to the next free one: eight groups never share a colour,
 *   and a group appended later moves none that is already drawn.
 * - **Everything below it is that one colour, at any depth** — its tasks, the groups inside
 *   it, their tasks, sessions and pictures. Shades of the group per sibling and per rank were
 *   tried and lost: on a real nested arrangement they read as a scatter of near-colours rather
 *   than as one branch, and telling siblings apart is what the cards are already for.
 * - **What is in no group is neutral**, all the way down: it has no category to show.
 *
 * Lightness and chroma are theme tokens (`--work-branch-*`); only the hue is computed here,
 * which keeps this section testable without a stylesheet.
 */
const BRANCH_HUES = [40, 85, 130, 175, 220, 265, 310, 355];
function branchHues(labels) {
  const taken = new Set(), hues = new Map();
  for (const label of labels) {
    let hash = 2166136261;
    for (const char of label) hash = Math.imul(hash ^ char.codePointAt(0), 16777619) >>> 0;
    hash = (hash ^ (hash >>> 16)) >>> 0;
    // Past eight groups every slot is taken and the probe comes back round to the label's own.
    let slot = hash % BRANCH_HUES.length;
    for (let i = 0; i < BRANCH_HUES.length && taken.has(slot); i++) slot = (slot + 1) % BRANCH_HUES.length;
    taken.add(slot);
    hues.set(label, BRANCH_HUES[slot]);
  }
  return hues;
}
/** Node id → the hue of the first-level group it sits under. Absent means in no group. */
function branchTones(model) {
  const children = childIndex(model);
  // Only a group on the core is first-level; a group inside a group wears its holder's colour.
  const groups = (children.get("core") || []).filter((n) => n.kind === "group").sort((a, b) => a.data.index - b.data.index);
  // By label, not title: the upkeep group's title is in the reader's language, its label is not.
  const hues = branchHues(groups.map((g) => g.data.label));
  const tones = new Map();
  const paint = (node, hue) => {
    tones.set(node.id, hue);
    for (const kid of children.get(node.id) || []) paint(kid, hue);
  };
  for (const group of groups) paint(group, hues.get(group.data.label));
  return tones;
}
/** A hue as CSS: the wire's lightness, or the label's darker one for text. Neutral without one. */
function branchPaint(hue, part = "wire") {
  if (hue === undefined) return part === "label" ? "var(--fg)" : "var(--work-line)";
  return `oklch(${part === "label" ? "var(--work-label-l)" : "var(--work-branch-l)"} var(--work-branch-c) ${hue})`;
}

/**
 * **A group wears the icon drawn for it, and the default until one is.** The icon is the
 * grouping mind's call, drawn as an edit of the default picture so every icon is one set
 * (`docs/arch/home.md#icons`); this surface only fetches it. Nothing here picks an icon
 * from a label — a guess from the words is exactly the inference the grouping record exists
 * to keep out of code.
 */
const DEFAULT_GROUP_ICON = "/api/home/group-icon";
function groupIcon(ref) {
  const rel = typeof ref === "string" && ref.startsWith("drive/") ? ref.slice("drive/".length) : "";
  return rel ? `/api/drive/file/${rel.split("/").map(encodeURIComponent).join("/")}` : DEFAULT_GROUP_ICON;
}

/**
 * HomeModel is a read projection, NEVER another task/session lifecycle store.
 *
 * HomeNode = { id, kind, title, summary?, sourceRefs: SourceRef[], data: NodeData }
 * NodeData is discriminated by kind:
 * - core: { sessions: SessionState[], overviewIds: string[] }
 * - task: { task: TaskDto, status, endedAt, results: Result[], sessions: SessionState[] }
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
 * 3. Reaction/Cognition sessions compose the one core. A session working on a drawn task is
 *    that task's `data.sessions` — one line on its card, never a card of its own. The rest are
 *    activity cards: Reflection and every session with no subject in the built-in upkeep
 *    group, and a session whose task is not drawn on the core.
 * 4. subject is the authoritative session -> task join. owner is a technical session
 *    relationship only; it must not pretend that two independent tasks are one piece of work.
 * 5. Overview uses useMessages()'s USER-VISIBLE transcript and factual task transitions.
 *    No registry tail, raw reasoning, or tool log is used to manufacture a public plan.
 * 6. A task hangs off the core, or off the innermost group the person's arrangement puts it
 *    in. Nothing infers a group — see below.
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
  // `refs` is what this task MADE, newest first: the store's `made` lines, which it writes
  // when a session serving the task renders a view (`view_refs` in
  // `foundation/server/tasks.rs`). The server has already dropped views no longer on disk,
  // the app's own surfaces and builders' `_` probes, so this only looks each one up.
  //
  // It used to be every known view the task's prose MENTIONED, and a mention has no verb:
  // "the screen is showing `research-two-pairs`" hung a shoe report under a KTV task. The
  // system-view filter that lived here was the first patch on that, for the one class a flag
  // could catch; the fix is at the source, and the patch went with the cause.
  //
  // **Only a picture is kept.** A result without one — a view never shot, a file the task
  // wrote — used to feed a count printed on the card, and the count is gone: a number with
  // nothing to open behind it is a claim the card cannot back, and `factory/tasks` lists them.
  const byRef = new Map(views.filter((v) => v.view_ref && v.shot_url).map((v) => [v.view_ref, v]));
  return (task.refs || []).filter((r) => byRef.has(r)).map((r) => byRef.get(r))
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
 * **Order decides which six: the newest this task made.** That is the time on the task's own
 * `made` line. View records still have no timestamp, and the `?v=` cache-buster on the shot URL
 * is not declared to mean recency, so the tiles are the first shot-bearing views in `refs`
 * order and claim nothing about when a view last changed.
 */

/**
 * **A group is a name over some tasks and groups, and this surface is the only thing that
 * knows it.**
 *
 * It is not a field on a task: that would make one view's axis — project, or kind, or state —
 * everybody's, and the axis is the person's to change. `/api/home/groups` serves what they
 * have arranged (`docs/arch/home.md#grouping`); nothing here matches, guesses or falls back
 * to a field that looks close. The rank that came before this one was inferred from
 * `systems`, which says what a task touches, and drew a birthday deck under "feishu".
 *
 * A group that ends up with no drawn task is not a node. Tasks close and age off this
 * surface while the record still names them, and an empty heading is structure standing
 * where its content used to be — at any depth, since a group is drawn only on the way to a
 * task drawn inside it.
 *
 * **A group can hold groups**, so what a task maps to is the chain of groups it sits in,
 * outermost first. The record's rules are the writer's, read the same way here so the two
 * agree about a record written before a rule existed: a group's own members are claimed
 * before the groups inside it, the first claim wins, and a label is one group — a second
 * group carrying a label already seen is left out whole, rather than hung under two parents.
 */
function groupIndex(groups) {
  const byTask = new Map();
  const labels = new Set();
  const walk = (list, chain) => (Array.isArray(list) ? list : []).forEach((group, index) => {
    const label = plain(group?.label);
    if (!label || labels.has(label)) return;
    labels.add(label);
    const here = [...chain, { label, note: plain(group.note), icon: plain(group.icon), index }];
    for (const subject of group.members || []) {
      const key = plain(subject);
      if (key && !byTask.has(key)) byTask.set(key, here);
    }
    walk(group.groups, here);
  });
  walk(groups, []);
  return byTask;
}

function buildHome({ tasks = [], workers = [], views = [], messages = [], groups = [] }, now = Date.now()) {
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
  const grouped = groupIndex(groups);
  for (const task of tasks) {
    // **A live session no longer re-admits its expired task.** That rule kept a closed task
    // present "as context" whenever anything recent still named it, and it was the single
    // biggest leak: fifteen of the twenty-five closed tasks on the canvas arrived that way,
    // the oldest closed twenty-six days earlier, each drawn as a peer of the work in hand.
    // A session whose task has aged out now connects to the core and keeps its own title.
    if (!OPEN.has(task.status) && !recent(taskEnd(task), now)) continue;
    const node = add({ id: taskKey(task.subject), kind: "task", title: task.title || task.subject,
      sourceRefs: [ref("task", task.subject)], data: { task, status: task.status,
        endedAt: taskEnd(task), results: taskResults(task, views), sessions: [] } });
    // A task hangs off its group when the arrangement puts it in one, and off the core when
    // it doesn't. Ungrouped is an ordinary place to be — see `groupIndex` for why nothing
    // here invents a group for a task the person has not placed.
    // A task inside an inner group draws every group on the way to it, each once however
    // many tasks pass through it — `add` and `link` both dedupe by id.
    const chain = grouped.get(task.subject);
    let parent = "core";
    for (const group of chain || []) {
      const id = `group:${group.label}`;
      add({ id, kind: "group", title: group.label, sourceRefs: [],
        data: { label: group.label, note: group.note, icon: group.icon, index: group.index } });
      link(parent, id);
      parent = id;
    }
    link(parent, node.id);
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
    // **A session working on a drawn task is that card's own state, not a card beside it.** The
    // two cards carried one fact between them: the session's title is the errand as it was handed
    // out, which is mostly the task's title said again, and the session's own contribution is
    // whether anybody is on it right now. A whole column for that.
    if (taskId) {
      const task = byId.get(taskId);
      task.data.sessions.push(session);
      task.sourceRefs.push(ref("session", session.id));
      continue;
    }
    // A subject whose task is not drawn stays on the core: that is somebody's work, aged out.
    const upkeep = session.role === "reflection" || !session.subject;
    if (upkeep) {
      add({ id: UPKEEP, kind: "group", title: L.upkeep, sourceRefs: [],
        data: { label: UPKEEP, note: L.upkeepNote, icon: "", index: Number.MAX_SAFE_INTEGER, builtin: true } });
      link("core", UPKEEP);
    }
    add({ id: session.id, kind: "activity", title: session.role === "reflection" ? L.roles.reflection : session.title,
      sourceRefs: [ref("session", session.id)],
      data: { session, taskId: null, ownerSessionId: session.ownerSessionId, currentAction: session.currentAction } });
    link(upkeep ? UPKEEP : "core", session.id, "contains");
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
  // **Inside a group: its own tasks, then its groups in the record's order.** Tasks keep the
  // order they were drawn in. The groups inside it follow the record rather than whichever of
  // their tasks happened to be drawn first. The core's branches are ordered by `arrange`.
  const inner = (n) => (n.kind === "group" ? 1 : 0);
  for (const [id, list] of children) {
    if (nodes.get(id)?.kind !== "group") continue;
    list.sort((a, b) => inner(a) - inner(b) || (inner(a) ? a.data.index - b.data.index : 0));
  }
  return children;
}

/**
 * **Whether the person is wanted on this task**: it is open and the newest line a mind wrote
 * on it is a `waiting` one. The same rule as `factory/tasks`' `waitsOnPerson`, over the same
 * `latest` field, so the card and the board cannot disagree.
 *
 * It is the card's status word because it is the one status the person has to act on, and
 * Home is where somebody comes back to once the conversation has stopped telling them where
 * each thing got to (`docs/arch/legibility.md` § F). "In progress" on a row that is waiting
 * on them says the opposite of what is true. It wins over a cut turn below, which says our
 * side stopped: a row whose last step is theirs is not one anybody is progressing.
 */
function waitsOnPerson(task) {
  return OPEN.has(task.status) && task.latest?.kind === "waiting";
}

/**
 * **One word, because the two states are not independent.** A card used to be able to carry a
 * ledger status and a session state at once — `To do` above `Working`, `Completed` above `Idle` —
 * and most of that grid does not exist: nothing is being worked on while it is still to do, and
 * a closed row is closed whatever is still warm. What the session actually adds to a row that is
 * open is whether anybody is on it *now*, which is a mark beside the word, not a word of its own.
 *
 * The one thing a session says that the ledger cannot is that **its last turn failed or was cut
 * off**: the row reads in progress and nothing is progressing. That replaces the word, and only
 * while nothing else is running on the same row.
 */
function stateOf(node) {
  if (node.kind === "task") {
    if (waitsOnPerson(node.data.task)) return "needsYou";
    const sessions = node.data.sessions || [];
    if (!OPEN.has(node.data.status) || sessions.some((s) => s.state === "running")) return node.data.status;
    const cut = sessions.find((s) => ["failed", "interrupted"].includes(s.lastTurn?.outcome));
    return cut ? cut.lastTurn.outcome : node.data.status;
  }
  if (node.kind !== "activity") return node.kind;
  const s = node.data.session;
  if (s.state !== "running" && ["failed", "interrupted"].includes(s.lastTurn?.outcome)) return s.lastTurn.outcome;
  return s.state;
}

/**
 * The live sessions on a task, running first: one dot each, filled while it runs. **Three at
 * most**, because the question a glance asks is whether anybody is on it, and the count is the
 * rest of the answer; past three the rest is a number. Their titles and states are the line's
 * hover text, and `factory/workers` has all of it.
 */
const HANDS = 3;
function hands(node) {
  const sessions = [...(node.data.sessions || [])].sort((a, b) =>
    (b.state === "running") - (a.state === "running") || String(b.stateSince).localeCompare(String(a.stateSince)));
  return { shown: sessions.slice(0, HANDS), more: Math.max(0, sessions.length - HANDS),
    title: sessions.map((s) => `${s.title} · ${L.status[s.state] || s.state}`).join("\n") };
}

function age(value, now) {
  const at = instant(value);
  if (at === null) return L.unknown;
  const minutes = Math.max(0, Math.floor((now - at) / 60000));
  return minutes < 60 ? L.ago(minutes, "m") : minutes < 1440 ? L.ago(Math.floor(minutes / 60), "h") : L.ago(Math.floor(minutes / 1440), "d");
}

function nodeTime(node) {
  // A card's time is how long its status word has held, so a wait is timed from its line.
  if (node.kind === "task" && waitsOnPerson(node.data.task)) return node.data.task.latest.at || node.data.task.statusSince;
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
 * At 1x, a 48px gutter preserves the curve without spending a card's width on empty ranks.
 */
const GAP_X = 48, MARGIN = 24;
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
/**
 * **A group is a label, not a card**, and that is the whole of its appearance: it holds no
 * status, no time and no board to hand off to, so a card-sized box would promise all three. Narrow
 * also costs the rank it adds the least width — the chart already runs wider than a laptop.
 */
const GROUP_W = 144, GROUP_H = 56;
/** A group taken as the centre stands where the core was: the same heading, a size up. */
const FOCUS_W = 240, FOCUS_H = 80;
function dimensions(node) {
  return node?.kind === "group" ? { w: GROUP_W, h: GROUP_H } : { w: CARD_W, h: CARD_H };
}

/**
 * **Home opens at 1x the first time, and after that at the scale this window left it at.** It
 * used to open fitted to the window with a floor of 0.7, and the floor was where every real
 * day landed: fitting shrank
 * every card to 70% — a 17px title drawn at 12px, a 16:9 picture at 168x95 — to buy the one
 * view of the whole chart that nobody was reading at that size. A day that is wider or taller
 * than the window scrolls from the core outward instead, and pinch or ⌘/Ctrl-wheel is there
 * for the moment the shape of the whole is what someone wants.
 *
 * The range reaches out far enough to see that shape and in far enough to read a picture.
 */
const ZOOM_MIN = 0.25, ZOOM_MAX = 2;
const clampZoom = (scale) => Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, scale));

/**
 * **Every point of the chart can be brought to the middle of the window**, so the drawing
 * sits inside half a window of air on every side.
 *
 * The canvas used to be the drawing and no more, padded only far enough to centre the core.
 * A card on the chart's outer edge could be scrolled no further than the window's edge, so
 * zooming in on one pinned it there, half off screen, with no way to drag it to where it
 * could be read. Half a frame each side is exactly the room that makes every edge reachable,
 * and it centres the core — or a focused group — without a rule of its own.
 */
function stage(chart, frame, scale) {
  const drawn = { w: chart.width * scale, h: chart.height * scale };
  return { canvas: { w: Math.ceil(drawn.w + frame.w), h: Math.ceil(drawn.h + frame.h) },
    offset: { x: frame.w / 2, y: frame.h / 2 } };
}

/** The scroll that keeps the chart point under `point` (window pixels) there across a zoom. */
function zoomAround(chart, frame, from, to, scroll, point) {
  const before = stage(chart, frame, from).offset, after = stage(chart, frame, to).offset;
  const x = (scroll.left + point.x - before.x) / from, y = (scroll.top + point.y - before.y) / from;
  return { left: after.x + x * to - point.x, top: after.y + y * to - point.y };
}

/**
 * **A group can be taken as the centre**, and then the chart is that group's branch and
 * nothing else: the group where the core was, with its tasks, inner groups, sessions and
 * pictures around it. Null when `id` is not a group this model draws — a focus kept from an
 * earlier open whose group has since closed to nothing is the whole chart again, never an
 * empty one.
 *
 * It is a lens, not a collapse: the same model cut at a node, so every rule that places a card
 * places it the same way here. Like the scroll, the focus belongs to the window.
 */
function focusOn(model, id) {
  const root = id ? model.nodes.find((n) => n.id === id) : null;
  if (!root || root.kind !== "group") return null;
  const children = childIndex(model), keep = new Set();
  const walk = (node) => { keep.add(node.id); for (const kid of children.get(node.id) || []) walk(kid); };
  walk(root);
  return { ...model, rootId: id, nodes: [root, ...model.nodes.filter((n) => keep.has(n.id) && n !== root)],
    edges: model.edges.filter((e) => keep.has(e.from) && keep.has(e.to)) };
}

/** The nodes from the core's first child down to `id`, outermost first: the way back out. */
function trail(model, id) {
  const parent = new Map(model.edges.filter((e) => e.primary).map((e) => [e.to, e.from]));
  const nodes = new Map(model.nodes.map((n) => [n.id, n]));
  const path = [];
  for (let at = id; at && at !== model.rootId && nodes.has(at); at = parent.get(at)) path.unshift(nodes.get(at));
  return path;
}

/**
 * **Where the window was is a card, not a pixel.** The chart is laid out afresh on every open,
 * and a task filed since the last one moves every branch below it, so a remembered scroll
 * offset lands somewhere else on the next day's chart. What is kept instead is the card
 * nearest the middle of the window and how far the middle was from that card's centre, in
 * chart units; on the way back that card is put in the same place. A card that is gone falls
 * back to the root.
 */
function anchorAt(chart, point) {
  let best = null, bestDistance = Infinity;
  for (const row of chart.placed) {
    const cx = row.x + row.w / 2, cy = row.y + row.h / 2;
    // To the box, not its centre: anywhere inside the core's large box is the core.
    const distance = Math.hypot(Math.max(0, Math.abs(point.x - cx) - row.w / 2),
      Math.max(0, Math.abs(point.y - cy) - row.h / 2));
    if (distance < bestDistance) { best = row; bestDistance = distance; }
  }
  return best && { id: best.node.id, dx: point.x - (best.x + best.w / 2), dy: point.y - (best.y + best.h / 2) };
}
function anchorPoint(chart, anchor) {
  const row = (anchor && chart.placed.find((p) => p.node.id === anchor.id)) || null;
  const at = row || chart.placed[0];
  return { x: at.x + at.w / 2 + (row ? Number(anchor.dx) || 0 : 0),
    y: at.y + at.h / 2 + (row ? Number(anchor.dy) || 0 : 0) };
}

/**
 * Semantic edges determine the hierarchy; flextree only computes its geometry.
 * Overview children are embedded INSIDE the core, not duplicated as peripheral cards.
 *
 * **The whole tree is drawn** unless the person has taken a group as the centre (`focusOn`).
 * There is no collapse, because with the work in hand and nothing else there is nothing to
 * hide from: the instance that laid out 225 cards over 2556x16529px lays out 20.
 *
 * `tones` are the whole model's, so a focused branch keeps the colour it wears on the whole
 * chart instead of going neutral for want of a core above its group.
 */
function arrange(model, tones = branchTones(model)) {
  const children = childIndex(model);
  const root = model.nodes.find((n) => n.id === model.rootId);
  const rootBox = root.kind === "core" ? { w: CORE_W, h: CORE_H } : { w: FOCUS_W, h: FOCUS_H };
  const branches = children.get(model.rootId).filter((n) => n.kind !== "overview");
  const tree = (node) => ({ node, ...dimensions(node),
    children: (children.get(node.id) || []).map(tree) });
  const weight = (t, rank = 1) => Math.max(t.h,
    t.children.reduce((n, c) => n + weight(c, rank + 1) + gapAt(rank + 1), 0));
  const sides = [[], []], load = [0, 0];
  // **The arrangement's order for the groups, and ids for everything else.** A group is
  // where the person put it, because the record is an ordered list; an ungrouped task has
  // nobody's order to follow, so it keeps the id sort — ids, not activity states, are what
  // hold a branch still between updates. Which *side* either lands on is still the balance's
  // call, and that is the part a person cannot predict: `home.md` § Open.
  // A focused group's own children are already in the record's order (`childIndex`).
  const rank = (n) => (n.kind === "group" ? [0, n.data.index, ""] : [1, 0, n.id]);
  const inOrder = root.kind !== "core" ? branches : [...branches].sort((a, b) => {
    const [ak, ai, aid] = rank(a), [bk, bi, bid] = rank(b);
    return ak - bk || ai - bi || aid.localeCompare(bid);
  });
  for (const branch of inOrder) {
    const t = tree(branch), side = load[0] <= load[1] ? 0 : 1;
    sides[side].push(t); load[side] += weight(t);
  }
  const placed = [{ node: root, x: -rootBox.w / 2, y: -rootBox.h / 2, ...rootBox, dir: 0 }];
  for (let side = 0; side < 2; side++) {
    if (!sides[side].length) continue;
    // The node's own extent only; the air between two of them is `spacing`, which sees both
    // and so can ask where they parted. A padded `nodeSize` cannot — it is one number per node.
    const layout = flextree({ nodeSize: (n) => [n.data.h, n.data.w + GAP_X],
      spacing: (a, b) => gapAt(divergence(a, b)) });
    const hub = layout.hierarchy({ w: rootBox.w / 2, h: rootBox.h, children: sides[side] });
    layout(hub);
    const dir = side === 0 ? 1 : -1;
    for (const n of hub.descendants().slice(1)) {
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
    return { ...edge, paint: branchPaint(tones.get(edge.to)), d: `M ${sx} ${sy} C ${mid} ${sy}, ${mid} ${ey}, ${ex} ${ey}` };
  });
  return { placed, wires, width, height };
}

async function getJson(path) {
  const response = await fetch(path);
  if (!response.ok) throw new Error(`${response.status}`);
  return response.json();
}

/**
 * **Where this window was on Home — its scale, the group it took as the centre, the card in
 * the middle — is kept by the window, for the next time Home is opened in it.** It is not a
 * record and nothing else reads it: another device, or a phone beside this laptop, keeps its
 * own. A store that refuses (a private window, storage switched off) opens Home the way a
 * first visit does.
 */
const PLACE_KEY = "hi-home-place";
function readPlace() {
  try {
    const place = JSON.parse(localStorage.getItem(PLACE_KEY) || "null");
    return place && typeof place === "object" ? place : {};
  } catch { return {}; }
}
function writePlace(place) {
  try { localStorage.setItem(PLACE_KEY, JSON.stringify({ ...readPlace(), ...place })); } catch { /* opens fresh next time */ }
}

export default function Home() {
  const { openRef } = useViews();
  const { messages } = useMessages();
  const [source, setSource] = useState({ tasks: [], workers: [], views: [], groups: [] });
  const [loaded, setLoaded] = useState(false);
  const [ledgerSettled, setLedgerSettled] = useState(false);
  const [sourcesSettled, setSourcesSettled] = useState(false);
  const [frameMeasured, setFrameMeasured] = useState(false);
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
        // How the person has arranged the work, which is this surface's own record and
        // nobody else's. It changes when somebody says so, not on a clock, but it is small
        // and it rides the poll the ledger's neighbours are on anyway.
        ["groups", "/api/home/groups", (v) => v.groups],
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
      setNow(Date.now()); setLoaded(true); setSourcesSettled(true);
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
      setLedgerSettled(true);
      return null;
    }
    setErrors((prev) => prev.filter((source) => source !== "tasks"));
    if (answer.unchanged) {
      setLedgerSettled(true);
      return answer.version;
    }
    if (!Array.isArray(answer.tasks)) {
      setLedgerSettled(true);
      return null;
    }
    setSource((prev) => ({ ...prev, tasks: answer.tasks }));
    setLedgerSettled(true);
    setNow(Date.now());
    setLoaded(true);
    return answer.version;
  }, []);
  useWatched(readLedger);
  const model = useMemo(() => buildHome({ ...source, messages }, now), [source, messages, now]);
  const children = useMemo(() => childIndex(model), [model]);
  const tones = useMemo(() => branchTones(model), [model]);
  const kept = useRef(null);
  if (kept.current === null) kept.current = readPlace();
  const [focus, setFocus] = useState(() => (typeof kept.current.focus === "string" ? kept.current.focus : null));
  const focused = useMemo(() => focusOn(model, focus), [model, focus]);
  const shown = focused || model;
  const chart = useMemo(() => arrange(shown, tones), [shown, tones]);
  const path = useMemo(() => (focused ? trail(model, focus) : []), [model, focused, focus]);
  const mobile = frame.w < 760;
  const settled = sourcesSettled && ledgerSettled;
  // A focus whose group has closed to nothing is let go once the sources have answered, so the
  // group coming back later does not pull the window into it unasked.
  useEffect(() => {
    if (settled && focus && !focused) { setFocus(null); writePlace({ focus: null }); }
  }, [settled, focus, focused]);
  useEffect(() => {
    const el = viewport.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => {
      setFrame({ w: entry.contentRect.width, h: entry.contentRect.height });
      setFrameMeasured(entry.contentRect.width > 0 && entry.contentRect.height > 0);
    });
    observer.observe(el); return () => observer.disconnect();
  }, []);
  const [scale, setScale] = useState(() => clampZoom(Number(kept.current.scale) || 1));
  const { canvas, offset } = stage(chart, frame, scale);
  // Wheel and gesture events arrive faster than renders, so each zoom composes on the scale and
  // scroll the previous one asked for rather than on what is on screen yet.
  const live = useRef({ scale, chart, frame, focus }), pending = useRef(null);
  live.current = { scale: pending.current?.scale ?? scale, chart, frame, focus };
  // The place is read off the scroll as it moves, which is cheap — a nearest box among a few
  // dozen — and written a moment after it stops, and once more as Home goes away.
  const centred = useRef(false), place = useRef(null), writing = useRef(0);
  const remember = useCallback(() => {
    const el = viewport.current;
    if (!el || !centred.current) return;
    const { scale, chart, frame, focus } = live.current, { offset } = stage(chart, frame, scale);
    const middle = { x: (el.scrollLeft + frame.w / 2 - offset.x) / scale, y: (el.scrollTop + frame.h / 2 - offset.y) / scale };
    place.current = { scale, focus, anchor: anchorAt(chart, middle) };
    clearTimeout(writing.current);
    writing.current = setTimeout(() => { writePlace(place.current); writing.current = 0; }, 300);
  }, []);
  useEffect(() => () => {
    if (writing.current) { clearTimeout(writing.current); writePlace(place.current); }
  }, []);
  const scrollToPoint = (point) => viewport.current?.scrollTo({ left: offset.x + point.x * scale - frame.w / 2,
    top: offset.y + point.y * scale - frame.h / 2, behavior: "instant" });
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
    remember();
  }, [scale, remember]);
  // Place once the initial sources and viewport are ready: on the card this window was last
  // looking at, or on the centre the first time. Later updates keep the person's scroll.
  useLayoutEffect(() => {
    if (!settled || !frameMeasured || mobile || !viewport.current || centred.current) return;
    const el = viewport.current, style = getComputedStyle(el);
    const height = el.clientHeight - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom);
    const width = el.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
    // A newly displayed error banner can resize the viewport before ResizeObserver runs.
    if (Math.abs(height - frame.h) > 1 || Math.abs(width - frame.w) > 1) return;
    // A kept focus is either drawn by now or about to be let go; place on the chart it settles to.
    if (focus && !focused) return;
    centred.current = true;
    scrollToPoint(anchorPoint(chart, kept.current.anchor));
    remember();
  }, [settled, frameMeasured, mobile, chart, frame, scale, offset.x, offset.y, focus, focused, remember]);
  // Taking a group as the centre puts it in the middle; stepping back out puts the group just
  // left in the middle of the wider chart, so the person sees where it sits.
  const recentre = useRef(null);
  const centreOn = useCallback((next) => {
    const from = live.current.focus;
    if (next === from) return;
    const outward = next === null || trail(model, from).some((n) => n.id === next);
    recentre.current = { id: outward ? from : next, dx: 0, dy: 0 };
    setFocus(next);
    if (mobile) writePlace({ focus: next });
  }, [model, mobile]);
  useLayoutEffect(() => {
    if (!recentre.current || mobile || !centred.current) return;
    scrollToPoint(anchorPoint(chart, recentre.current));
    recentre.current = null;
    remember();
  });
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
  const common = { now, children, openRef, tones, centreOn };
  // One step out from a focused group: the group holding it, or the whole chart.
  const up = path.length > 1 ? path[path.length - 2].id : null;
  return (
    <div className="hi-work">
      <style>{CSS}</style>
      {errors.length > 0 && <div className="hi-work__error" role="status">{L.failed}: {errors.map((key) => L.source[key]).join(" · ")}
        <button onClick={refresh}>{L.retry}</button></div>}
      <div className="hi-work__frame">
      {/* The way back out, and only while there is somewhere to go back to: the whole chart has
          no chrome over it, the way it has no zoom control. */}
      {focused && <nav className="hi-work__trail" aria-label={L.trail}>
        <button onClick={() => centreOn(null)}>{L.core}</button>
        {path.map((node, i) => <span key={node.id} className="hi-work__trail-step">
          <span aria-hidden="true">›</span>
          {i < path.length - 1 ? <button onClick={() => centreOn(node.id)}>{node.title}</button>
            : <strong aria-current="location">{node.title}</strong>}
        </span>)}
      </nav>}
      <div className="hi-work__viewport" ref={viewport} data-chart={mobile ? undefined : ""} data-focused={focused ? "" : undefined}
        onScroll={mobile ? undefined : remember} onPointerDown={(e) => {
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
          {focused ? <Node node={focused.nodes[0]} root up={up} {...common} />
            : <Core node={model.nodes[0]} model={model} now={now} />}
          <Branch nodes={focused ? children.get(focus) || [] : branches} parent={shown.rootId} {...common} />
        </div> : <div className="hi-work__canvas" style={{ width: canvas.w, height: canvas.h }}>
          <div className="hi-work__stage" style={{ left: offset.x, top: offset.y, width: chart.width, height: chart.height,
            transform: `scale(${scale})` }}>
            <svg className="hi-work__wires" width={chart.width} height={chart.height} aria-hidden>
              {chart.wires.map((wire) => <path key={wire.id} data-edge={wire.id} d={wire.d} style={{ stroke: wire.paint }} />)}
            </svg>
            {chart.placed.map((row) => <div key={row.node.id} className="hi-work__position"
              style={{ left: row.x, top: row.y, width: row.w, height: row.h }}>
              {row.node.kind === "core" ? <Core node={model.nodes[0]} model={model} now={now} />
                : <Node node={row.node} root={row.node.id === shown.rootId} up={up} {...common} />}
            </div>)}
          </div>
        </div>}
      </div>
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
function Node({ node, now, children, openRef, tones, centreOn, root = false, up = null }) {
  const state = stateOf(node), time = nodeTime(node);
  const board = node.kind === "task" ? TASK_BOARD : node.kind === "activity" ? SESSION_BOARD : null;
  // A tile is the one handoff that lands exactly where it points: `openRef` takes a view ref
  // natively, so this needs none of the targeting the board handoff is waiting on.
  if (node.kind === "result") return <article className="hi-work__tile" data-node-id={node.id} data-kind="result">
    <button onClick={() => openRef(node.data.viewRef)} title={node.title}>
      <img src={node.data.shot} alt={node.title} loading="lazy" />
    </button>
  </article>;
  // A group carries its own note as hover text — one line saying what the grouping was
  // based on, so the person reading the chart can see why these three are one thing.
  // **A group opens its own branch**, on this surface: pressed, it becomes the centre, and the
  // group at the centre pressed again steps back out one level.
  if (node.kind === "group") return <article className="hi-work__group" data-node-id={node.id}
    data-kind="group" data-root={root ? "" : undefined} style={{ "--group-tone": branchPaint(tones.get(node.id), "label") }}>
    <button onClick={() => centreOn(root ? up : node.id)}
      title={[root ? L.back : node.title, node.data.note].filter(Boolean).join(" · ")}>
      {/* A drawn icon whose file has gone since the last arrangement is the default again. */}
      <img className="hi-work__group-icon" src={groupIcon(node.data.icon)} alt="" aria-hidden="true"
        onError={(event) => { if (!event.currentTarget.src.endsWith(DEFAULT_GROUP_ICON)) event.currentTarget.src = DEFAULT_GROUP_ICON; }} />
      <span>{node.title}</span>
    </button>
  </article>;
  const onIt = node.kind === "task" ? hands(node) : null;
  const body = <>
    <span className="hi-work__node-title" title={node.title}>{node.title}</span>
    <div className="hi-work__node-foot"><span className="hi-work__node-state" title={onIt?.title || undefined}>
      {node.kind === "activity" && <i className="hi-work__live" data-live={node.data.session.state === "running"} />}
      {onIt?.shown.map((s) => <i key={s.id} className="hi-work__live" data-live={s.state === "running"} />)}
      {onIt?.more > 0 && <small className="hi-work__more">+{onIt.more}</small>}
      {L.status[state] || ""}</span>
      {["task", "activity"].includes(node.kind) && <time>{age(time, now)}</time>}</div>
  </>;
  return <article className="hi-work__node" data-node-id={node.id} data-kind={node.kind}
    style={{ "--node-tone": TONE[state] || TONE.todo, opacity: emphasis(node, now) }}>
    {board ? <button className="hi-work__open" onClick={() => openRef(board)}>{body}</button> : body}
  </article>;
}

/** The narrow flow's rail is its parent's colour and each tick its own, as a wire would be. */
function Branch({ nodes, parent, ...props }) {
  return <ul className="hi-work__branch" style={{ "--rail": branchPaint(props.tones.get(parent)) }}>
    {nodes.map((node) => <li key={node.id} style={{ "--tick": branchPaint(props.tones.get(node.id)) }}>
      <Node node={node} {...props} />
      {props.children.get(node.id)?.length > 0 && <Branch nodes={props.children.get(node.id)} parent={node.id} {...props} />}
    </li>)}</ul>;
}

const CSS = `
.hi-work { --bg: var(--bg-0); --work-line: color-mix(in srgb, var(--fg-mute) 46%, var(--bg)); --work-shadow:0 1px 2px #0000000a, 0 3px 10px #00000008; --work-branch-l:0.62; --work-label-l:0.48; --work-branch-c:0.1; height:100%; min-height:0; position:relative; display:flex; flex-direction:column; color:var(--fg); background:color-mix(in srgb, var(--fg) 2%, var(--bg)); padding-top:var(--hi-safe-top, 0px); font-family:var(--font-display, sans-serif); letter-spacing:0; }
.hi-work *, .hi-work *::before, .hi-work *::after { box-sizing:border-box; }
.hi-work button { font:inherit; color:inherit; background:none; border:0; padding:0; cursor:pointer; text-align:left; }
.hi-work button:focus-visible { outline:2px solid var(--accent); outline-offset:2px; }
.hi-work__error { padding:8px 24px; color:var(--danger); font-size:13px; display:flex; align-items:center; gap:12px; }
.hi-work__error button { text-decoration:underline; min-height:36px; }
.hi-work__frame { flex:1; min-height:0; position:relative; display:flex; flex-direction:column; }
.hi-work__viewport { flex:1; min-height:0; overflow:auto; overscroll-behavior:contain; position:relative; touch-action:pan-x pan-y; padding-bottom:100px; }
.hi-work__viewport[data-chart] { touch-action:none; cursor:grab; }
.hi-work__viewport[data-chart]:active { cursor:grabbing; }
.hi-work__canvas { position:relative; user-select:none; -webkit-user-select:none; }
.hi-work__canvas img { -webkit-user-drag:none; }
.hi-work__stage { position:absolute; transform-origin:0 0; }
.hi-work__wires { position:absolute; left:0; top:0; pointer-events:none; }
@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) .hi-work { --work-branch-l:0.7; --work-label-l:0.8; --work-branch-c:0.09; } }
:root[data-theme="dark"] .hi-work { --work-branch-l:0.7; --work-label-l:0.8; --work-branch-c:0.09; }
.hi-work__wires path { fill:none; stroke-width:2.2; }
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
.hi-work__node, .hi-work__core, .hi-work__tile { box-shadow:var(--work-shadow); }
/* A heading, not a card: no border and no background, because it is a name over the cards
   below it rather than a thing beside them. */
.hi-work__group { height:100%; font-size:17px; line-height:1.4; font-weight:600; color:var(--group-tone); letter-spacing:0; }
.hi-work__group > button { width:100%; height:100%; display:flex; align-items:center; gap:8px; padding:0 4px; border-radius:10px; overflow:hidden; overflow-wrap:anywhere; transition:background-color 160ms ease; }
.hi-work__group > button:hover { background:color-mix(in srgb, var(--group-tone) 10%, transparent); }
.hi-work__group-icon { width:40px; height:40px; flex:0 0 40px; border-radius:10px; object-fit:cover; }
/* The group at the centre is the same heading a size up, standing where the core stood. */
.hi-work__group[data-root] { font-size:22px; }
.hi-work__group[data-root] > button { gap:12px; padding:0 8px; }
.hi-work__group[data-root] .hi-work__group-icon { width:56px; height:56px; flex-basis:56px; border-radius:14px; }
.hi-work__trail { position:absolute; top:12px; left:16px; z-index:3; max-width:calc(100% - 32px); display:flex; flex-wrap:wrap; align-items:center; gap:6px; padding:6px 14px; font-size:13px; line-height:1.4; color:var(--fg-mute); background:var(--bg); border:1px solid var(--work-line); border-radius:999px; box-shadow:var(--work-shadow); }
.hi-work__trail-step { display:inline-flex; align-items:center; gap:6px; min-width:0; }
.hi-work__trail button { color:var(--fg-mute); }
.hi-work__trail button:hover { color:var(--fg); text-decoration:underline; text-underline-offset:3px; }
.hi-work__trail strong { color:var(--fg); font-weight:600; }
.hi-work__group span { min-width:0; display:-webkit-box; -webkit-line-clamp:2; -webkit-box-orient:vertical; overflow:hidden; text-wrap:balance; }
.hi-work__open, .hi-work__node { padding:10px 14px; }
.hi-work__tile { height:100%; border:1px solid var(--work-line); border-radius:6px; overflow:hidden; background:color-mix(in srgb, var(--fg-mute) 10%, transparent); }
.hi-work__tile button { display:block; width:100%; height:100%; padding:0; }
.hi-work__tile img { display:block; width:100%; height:100%; object-fit:cover; object-position:top center; }
.hi-work__open { display:flex; flex-direction:column; flex:1; min-width:0; height:100%; padding:0; }
.hi-work__node-title { display:-webkit-box; -webkit-line-clamp:3; -webkit-box-orient:vertical; overflow:hidden; font-size:17px; line-height:1.4; overflow-wrap:anywhere; font-weight:500; }
.hi-work__node-foot { display:flex; flex-wrap:wrap; justify-content:space-between; gap:6px; margin-top:auto; padding-top:8px; font-size:12px; line-height:1.4; color:var(--fg-dim, var(--fg-mute)); }
.hi-work__node { transition:border-color 160ms ease; }
.hi-work__node:has(button:hover), .hi-work__node:focus-within { border-color:var(--fg-mute); }
@media (prefers-reduced-motion:reduce) { .hi-work__node, .hi-work__group > button { transition:none; } }
.hi-work__node-foot time { white-space:nowrap; }
.hi-work__node-state { display:flex; align-items:center; gap:6px; color:var(--node-tone); }
/* The hands on a task sit in its one status line: a dot each, filled while one runs. */
.hi-work__node-state .hi-work__live + .hi-work__live { margin-left:-3px; }
.hi-work__more { color:var(--fg-mute); font-size:11px; margin-right:2px; }
.hi-work__flow { max-width:720px; margin:0 auto; padding:20px 16px 80px; }
.hi-work__viewport[data-focused] .hi-work__flow { padding-top:64px; }
.hi-work__flow > .hi-work__group { height:auto; min-height:64px; }
.hi-work__flow .hi-work__core { height:auto; min-height:320px; }
/* The narrow flow grades its air by rank for the same reason the chart does: nesting alone
   put a task's own results as far from it as the next task's were. Inheriting --rank-gap
   carries the tightest value on down, so a fourth rank is no looser than the third. */
.hi-work__branch { --rank-gap:30px; list-style:none; padding:0 0 0 18px; margin:0 0 0 8px; border-left:2px solid var(--rail, var(--work-line)); }
.hi-work__branch .hi-work__branch { --rank-gap:18px; margin-left:0; padding-left:12px; }
.hi-work__branch .hi-work__branch .hi-work__branch { --rank-gap:10px; }
.hi-work__branch li { position:relative; padding-top:var(--rank-gap); min-width:0; }
.hi-work__branch li::before { content:''; position:absolute; width:18px; left:-18px; top:calc(var(--rank-gap) + 32px); border-top:2px solid var(--tick, var(--work-line)); }
.hi-work__branch .hi-work__node { min-height:116px; }
.hi-work__branch .hi-work__group { height:auto; min-height:28px; }
.hi-work__branch .hi-work__tile { height:auto; aspect-ratio:16 / 9; max-width:240px; }
.hi-work__branch .hi-work__branch li::before { left:-12px; width:12px; }
.hi-work__loading { position:absolute; left:24px; top:8px; color:var(--fg-mute); font-size:13px; z-index:2; }
@media (max-width:759px) { .hi-work__node-title { min-height:36px; } }
`;
