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
import { useLive, useWatched, useMessages, useViews, TEMPO, AttachmentPreview } from "@hi/core";
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
    source: { tasks: "tasks", workers: "live sessions", groups: "grouping" },
    shown: { picture: "Picture", clip: "Clip", view: "Page" },
    trail: "Where this branch sits", back: "Step back out", unopened: "Made while you were on another page, not opened yet",
    ago: (n, unit) => `${n}${unit} ago`,
    more: (n) => `${n} more`,
    onBoard: (n) => `${n} more on the task board`,
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
    source: { tasks: "任务", workers: "在线会话", groups: "分组" },
    shown: { picture: "图片", clip: "视频", view: "页面" },
    trail: "这一支所在的位置", back: "退回上一层", unopened: "做好时你在看别的页，还没打开",
    ago: (n, unit) => `${n}${{ m: "分钟", h: "小时", d: "天" }[unit]}前`,
    more: (n) => `还有 ${n} 项`,
    onBoard: (n) => `还有 ${n} 项，在任务板上`,
  },
};
const L = typeof document !== "undefined" && /^zh/i.test(document.documentElement.lang || navigator.language)
  ? COPY.zh : COPY.en;
/**
 * How long a closed card takes to fade to its dimmest, which is no longer how long it is kept
 * — see `keepsClosed`. The two were one constant when retention was one clock, and a card that
 * is kept because the work it serves is still open should not read as a day-old leftover for
 * the six days the ceiling allows it.
 */
const FADE_MS = 24 * 3600000;
/** How long a collected notice is kept past its collection, and the hard bound on any closed
 *  row. Both belong to `keepsClosed`, which is where what they mean is written down. */
const GRACE_MS = 1 * 3600000;
const CEILING_MS = 7 * 24 * 3600000;
/** How long a cancellation is on Home at all. It belongs to `onHome`. */
const CANCELLED_MS = 1 * 3600000;
const CORE_ROLES = new Set(["reaction", "cognition"]);
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
/** Where a card hands off. Home owns no detail of its own: a task's panel is the board's, drawn
 *  here (`loadTaskPanel`), and a session's is still its board. */
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
 * - **Every branch on the core takes one of eight hues**, evenly spaced around OKLCH at one
 *   lightness and one chroma, so any set of them sits together. The key picks its slot by
 *   hash and a taken slot moves on to the next free one: eight branches never share a colour.
 * - **A card in no group is a branch of its own, and is coloured like one.** Having no group
 *   is not having no meaning. It was drawn neutral once, in `--work-line`, the card's hairline,
 *   and that is a step off the *card*: once the ground was lifted clear of --bg the hairline
 *   landed on the ground's own lightness (0.919 on 0.933 light, 0.330 on 0.330 dark) and the
 *   wire of exactly the work nobody had filed yet could not be seen.
 * - **Groups are handed their hues first**, in the record's order, so a group appended later
 *   moves no group already drawn, and a card coming or going moves none. Ungrouped cards take
 *   what is left, work in hand before history, so the free hues go to what the window draws.
 * - **Everything below a branch is that one colour, at any depth** — its tasks, the groups
 *   inside it, their tasks, sessions and pictures. Shades of the group per sibling and per rank
 *   were tried and lost: on a real nested arrangement they read as a scatter of near-colours
 *   rather than as one branch, and telling siblings apart is what the cards are already for.
 * - **Only the core's own trunk is uncoloured** — the narrow flow's rail down from the core,
 *   which is no branch. It is the branch lightness at no chroma, the same weight of line.
 *
 * Lightness and chroma are theme tokens (`--work-branch-*`); only the hue is computed here,
 * which keeps this section testable without a stylesheet.
 */
const BRANCH_HUES = [40, 85, 130, 175, 220, 265, 310, 355];
function branchHues(keys) {
  const taken = new Set(), hues = new Map();
  for (const key of keys) {
    let hash = 2166136261;
    for (const char of key) hash = Math.imul(hash ^ char.codePointAt(0), 16777619) >>> 0;
    hash = (hash ^ (hash >>> 16)) >>> 0;
    // Past eight branches every slot is taken and the probe comes back round to the key's own.
    let slot = hash % BRANCH_HUES.length;
    for (let i = 0; i < BRANCH_HUES.length && taken.has(slot); i++) slot = (slot + 1) % BRANCH_HUES.length;
    taken.add(slot);
    hues.set(key, BRANCH_HUES[slot]);
  }
  return hues;
}
/** Node id → the hue of the branch on the core it sits under. Only the core has none. */
function branchTones(model) {
  const children = childIndex(model);
  // A group inside a group, or a card inside one, wears its holder's colour.
  const first = (children.get("core") || []).filter((n) => n.kind !== "overview");
  const groups = first.filter((n) => n.kind === "group").sort((a, b) => a.data.index - b.data.index);
  const loose = first.filter((n) => n.kind !== "group")
    .sort((a, b) => Number(inHand(b)) - Number(inHand(a)) || a.id.localeCompare(b.id));
  // A group by label, a card by id, which is what holds each still between updates.
  const hues = branchHues([...groups.map((g) => g.data.label), ...loose.map((n) => n.id)]);
  const tones = new Map();
  const paint = (node, hue) => {
    tones.set(node.id, hue);
    for (const kid of children.get(node.id) || []) paint(kid, hue);
  };
  for (const group of groups) paint(group, hues.get(group.data.label));
  for (const node of loose) paint(node, hues.get(node.id));
  return tones;
}
/** A hue as CSS: the wire's lightness, or the label's darker one for text. The core's trunk has none. */
function branchPaint(hue, part = "wire") {
  if (hue === undefined) return part === "label" ? "var(--fg)" : "oklch(var(--work-branch-l) 0 0)";
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
 * 1. TaskDto (/api/tasks) -> task:<subject>, EVERY row whatever its age, and whatever its
 *    status but one: a cancellation is a node for an hour (`onHome`). Status and time are
 *    copied, not inferred from sessions. For everything else what the clock decides is
 *    `inHand` — a tier, read by the window's cut — never whether the node exists.
 * 2. Registry Status (/api/workers) -> session:<run>:<id>. running means busy; waiting
 *    means QUEUED WORK, not waiting for the user; idle is still a LIVE session. Only live
 *    sessions are here at all — a session that has ended is `factory/workers`' subject.
 * 3. Reaction/Cognition sessions compose the one core. A session working on a drawn task is
 *    that task's `data.sessions`, and is drawn only where there is more than one of them: a
 *    card on the task's branch (`works-on`). The rest are activity cards: Reflection and a
 *    session with no subject in the group the arrangement marks `upkeep`, or on the core when
 *    none is; a session whose task is not drawn on the core. Code has no group of its own.
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
const recent = (value, now) => instant(value) !== null && instant(value) >= now - CEILING_MS;
const taskEnd = (task) => task.status === "done" ? task.completedAt || task.statusSince || null
  : task.status === "cancelled" ? task.cancelledAt || task.statusSince || null : null;
/**
 * **What keeps a closed card is the work it belongs to, not the clock.**
 *
 * A closed row is on a surface that draws the work in hand for one of two reasons, and they
 * are different reasons: it just finished and the person may not know yet (a notice), or its
 * result is still in play for something unfinished (context). One 24h window on closure time
 * was a coarse proxy for the first and no proxy at all for the second, so it did both jobs
 * badly — it held a batch of seven sweep-cancelled rows for a day, and it dropped a report
 * the person was still working against.
 *
 * `collected` is the person's own presence, and it is the transcript's, not a read receipt:
 * the first inbound message *after* this row closed. No client identity, no cursor, no
 * acknowledgement — `text.rs` has none of those and never will (`docs/arch/text-transcript.md`).
 * It must be per-row: measuring the grace from the *newest* inbound message instead lets one
 * message resurrect a week of history, which on the instance this was measured against turned
 * 14 closed cards into 37.
 *
 * **Seen is deliberately not the test, and the measurement is why.** The person's screen moves
 * *are* recorded durably — 509 `went to "<ref>"` observations on that instance — but the only
 * host-written task↔view join is the `made` line, and there were 17 of those across 12 of 208
 * tasks. A signal that can answer for 6% of rows cannot decide retention. Nor is seen the axis
 * the person asked for: the report they had already read is the one they wanted kept, because
 * the work it serves was still open.
 */
/**
 * The first inbound message after `end`, or null if the person has not been back since. A row
 * nobody has come back to is still a notice, so an absent one keeps the card: work that closes
 * while they sleep is still there when they wake, which the fixed window could not promise.
 *
 * The transcript is the live window of 200 (`transcript.rs`), so for a row older than the
 * oldest message kept, the earliest *retained* inbound stands in for the real one. That can
 * only make a collection look earlier than it was, never later — it biases an old row toward
 * leaving, which is the direction this surface already takes when it cannot tell.
 */
const collectedAt = (end, inbound) => {
  const at = instant(end);
  if (at === null) return null;
  for (const ts of inbound) if (ts > at) return ts;
  return null;
};
/**
 * Whether a closed row is still **in hand**, given its innermost group's own open work.
 *
 * **This decides rank, never existence.** Every row the ledger has is a node with its whole
 * branch — its groups, its pictures, its sessions — but a spent cancellation, which `onHome`
 * takes out before this is asked; and what this answers is whether the row
 * is part of what is going on right now or part of what the thread has been through. `heat`
 * turns that into a tier, `budgeted` fills the window from the top, and what is left over
 * is drawn where there is room for it. It used to be an admission gate in `buildHome`: a row
 * that failed it was never a node, so no lens downstream could reach it, and pressing into a
 * group could only ever take away. The rules below are unchanged; only what they decide is.
 *
 * `threadLive` is "an open task shares this row's innermost group" — the person's words for it
 * were *the parent has not disappeared*, and the innermost group is that parent. The first-level
 * branch is not: the merged-video row sat in `北控视频` with nothing else open while `KNQ` above
 * it was busy, and testing the branch would have kept it.
 *
 * **A cancellation is never kept by a thread**, and only ever reaches here inside its hour —
 * see `onHome` — where it is a notice whatever else is running beside it.
 */
function keepsClosed(task, end, { threadLive, inbound }, now) {
  if (!recent(end, now)) return false;
  if (task.status === "done" && threadLive) return true;
  const collected = collectedAt(end, inbound);
  return collected === null || now - collected < GRACE_MS;
}
/**
 * **A cancellation is on Home for an hour, and then it is not on Home at all.** This is the one
 * rule here that decides existence rather than rank, and it goes by what the row is, not by how
 * full the window is. Ranking it as history is what failed: a group of five duties drew two
 * cancellations three days closed because the window had room, and the person's word for them
 * was *pure interference — even with few nodes, nobody wants to see them*. It has nothing to
 * come back to — what it made on the way is process, and the row's own word says the work is
 * not happening — so neither a live sibling, nor nobody being back yet, nor room to spare keeps
 * it. The hour runs from the closure, not from the person's next message the way a notice's
 * grace does: it is long enough to see the ask landed, and nothing after it is worth the window.
 * After it the row is `factory/tasks`' alone, and a missing closure time reads as past it.
 */
const onHome = (task, now) => task.status !== "cancelled"
  || (instant(taskEnd(task)) ?? -Infinity) >= now - CANCELLED_MS;
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

function taskResults(task) {
  // `attached` is what this task's lines PLACED, newest first by the line that placed each
  // (`shown` in `foundation/server/tasks.rs`): the views it made — the store's `made` lines,
  // written when a session serving the task renders one — and the pictures and clips its lines
  // carried, handed over through `hi_task_note` (`docs/arch/showing.md`). The server has
  // already dropped views no longer on disk, the app's own surfaces, builders' `_` probes and
  // any attachment id nothing placed, so this only reshapes them.
  //
  // It used to be every known view the task's prose MENTIONED, and a mention has no verb:
  // "the screen is showing `research-two-pairs`" hung a shoe report under a KTV task. A path a
  // line spells is the same mention and is not read either.
  //
  // **A view with no picture yet is kept**, drawn as its name until the server's warm-up
  // lands one. It used to be dropped, and a view a builder had only reviewed — never shown —
  // has none, so a task's result was hidden until somebody happened to open it.
  return (task.attached || []).map((item) => ({
    id: item.ref,
    kind: item.kind,
    title: item.label || L.shown[item.kind] || item.kind,
    preview: item.preview || null,
    viewRef: item.kind === "view" ? item.ref.slice("view:".length) : null,
    at: item.at || null,
    durationMs: item.durationMs ?? null,
  }));
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
 * twenty-one cards for them were 45% of the old canvas. What hangs below is a view the work
 * made or a picture or clip one of its lines carried; what it wrote besides is
 * `factory/tasks`' to list.
 *
 * **Order decides which six: the newest this task placed.** That is the time on the line that
 * placed each — its `made` line, or the line that carried it. The server has already cut the
 * row to six in that order, and a view record still has no clock of its own.
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
 *
 * `upkeep` is the chain to the group the record marks as holding the agent's own upkeep —
 * sessions with no row, so no member can name them — read the same way, or null.
 */
function groupIndex(groups) {
  const byTask = new Map();
  const labels = new Set();
  let upkeep = null;
  const walk = (list, chain) => (Array.isArray(list) ? list : []).forEach((group, index) => {
    const label = plain(group?.label);
    if (!label || labels.has(label)) return;
    labels.add(label);
    const here = [...chain, { label, note: plain(group.note), icon: plain(group.icon), index }];
    for (const subject of group.members || []) {
      const key = plain(subject);
      if (key && !byTask.has(key)) byTask.set(key, here);
    }
    if (group.upkeep === true && !upkeep) upkeep = here;
    walk(group.groups, here);
  });
  walk(groups, []);
  return { byTask, upkeep };
}

function buildHome({ tasks = [], workers = [], messages = [], groups = [] }, now = Date.now()) {
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
  const { byTask: grouped, upkeep: upkeepChain } = groupIndex(groups);
  // Every group on the way to a card, each once however many cards pass through it — `add` and
  // `link` both dedupe by id. Answers the innermost, which is the card's parent.
  const hang = (chain) => {
    let parent = "core";
    for (const group of chain || []) {
      const id = `group:${group.label}`;
      add({ id, kind: "group", title: group.label, sourceRefs: [],
        data: { label: group.label, note: group.note, icon: group.icon, index: group.index } });
      link(parent, id);
      parent = id;
    }
    return parent;
  };
  // The person's own presence, oldest first, for `keepsClosed`. The transcript is already a
  // parameter here — the overview below reads the same list — so a closed card's fate costs no
  // record, no endpoint and no client identity that the text channel has refused to carry.
  const inbound = messages.filter((m) => m.role === "user" && instant(m.ts) !== null)
    .map((m) => instant(m.ts)).sort((a, b) => a - b);
  // Which groups still hold open work, by innermost label. Built from the arrangement and the
  // ledger together: the arrangement says what a row belongs with, the ledger says what is
  // still running, and neither alone can answer whether a thread is finished.
  const liveGroups = new Set();
  for (const t of tasks) {
    if (!OPEN.has(t.status)) continue;
    const chain = grouped.get(t.subject);
    if (chain?.length) liveGroups.add(chain[chain.length - 1].label);
  }
  for (const task of tasks) {
    // **Every row in the ledger is a node, with the whole of its branch.** What the clock and
    // the person's presence decide is `inHand` — whether this row is what is going on now —
    // and that is a tier in `heat`, not a gate here. A row that is not in hand is history: it
    // is drawn where a branch has room for it, and pressing into a group is how somebody asks
    // for it. As a gate this dropped the row before it was ever a node, which is why the lens
    // downstream could only ever take away.
    //
    // It is still the one thing that separates the work in hand from the record of it, so it
    // is also what the core's updates are built from and what a group's *N more* counts.
    //
    // The one row that is not a node is a cancellation past its hour, and it takes its
    // pictures with it: a group left holding nothing else is never added.
    if (!onHome(task, now)) continue;
    const chain = grouped.get(task.subject);
    const threadLive = !!chain?.length && liveGroups.has(chain[chain.length - 1].label);
    const inHand = OPEN.has(task.status)
      || keepsClosed(task, taskEnd(task), { threadLive, inbound }, now);
    const node = add({ id: taskKey(task.subject), kind: "task", title: task.title || task.subject,
      sourceRefs: [ref("task", task.subject)], data: { task, status: task.status, inHand,
        endedAt: taskEnd(task), results: taskResults(task), sessions: [] } });
    // A task hangs off its group when the arrangement puts it in one, and off the core when
    // it doesn't. Ungrouped is an ordinary place to be — see `groupIndex` for why nothing
    // here invents a group for a task the person has not placed.
    link(hang(chain), node.id);
    // Pictures are second-level, the way a sub-step or a sub-result is — all of them, so that
    // one kind of record has one appearance. A task mid-flight often has process pictures and
    // no deliverable yet, and nothing here claims to know which of them is which.
    for (const result of node.data.results.slice(0, RESULT_TILES)) {
      const id = `result:${result.id}`;
      const exists = byId.has(id);
      add({ id, kind: "result", title: result.title,
        sourceRefs: [ref(result.viewRef ? "view" : "attachment", result.id)],
        data: { ...result, subject: task.subject } });
      link(node.id, id, "produces", !exists);
    }
  }
  for (const session of activities) {
    // **A live session still does not re-admit its own expired task.** The row is in the model
    // now — everything is — but a session hangs off a task only while that task is in hand;
    // otherwise it connects to the core and keeps its own title, exactly as before. Hanging it
    // under a row that is history would make the row drawable through its child, which is the
    // leak this rule was written for: fifteen of twenty-five closed tasks on the old canvas
    // arrived that way, the oldest closed twenty-six days earlier. The join itself is not lost
    // — the session's `subject` still names it, and `factory/workers` has both ends.
    const host = session.subject ? byId.get(taskKey(session.subject)) : null;
    const taskId = host?.data.inHand ? host.id : null;
    // A session working on a drawn task belongs to that task. Whether it is a word on the row
    // or a card below it is decided once the count is known — see the pass after this loop.
    if (taskId) {
      const task = byId.get(taskId);
      task.data.sessions.push(session);
      task.sourceRefs.push(ref("session", session.id));
      continue;
    }
    // **The agent's own upkeep — Reflection, and a session dispatch gave no subject — goes where
    // the arrangement puts it, and code draws no group of its own for it.** It drew one once,
    // and a person whose arrangement already had a group for that saw one category under two
    // headings. With no group marked for it, it is on the core like somebody's aged-out work.
    const upkeep = session.role === "reflection" || !session.subject;
    add({ id: session.id, kind: "activity", title: session.role === "reflection" ? L.roles.reflection : session.title,
      sourceRefs: [ref("session", session.id)],
      data: { session, taskId: null, ownerSessionId: session.ownerSessionId, currentAction: session.currentAction } });
    link(upkeep ? hang(upkeepChain) : "core", session.id, "contains");
  }
  // **More than one hand on a task draws them; one hand draws nothing.** They were dots on the
  // task's status line, filled while running — three at most and a `+1` past that — and the dot
  // was the only thing that said anybody was on it. A mark is a poor way to say a thing a word
  // can say: the row read `On duty` with two hollow marks in front of it, and a reader had to
  // have been told what a hollow mark was before the card said anything at all; the sessions'
  // own titles were hover text, which on a touch screen is nothing. So they are cards on the
  // `works-on` branch the model has always carried, each with its own title, word and time.
  //
  // **The lone session is the one that earns no card**, and this is not a special case bolted
  // on: it is the same reason session cards came off the chart as peers in the first place. A
  // session's title is the errand as it was handed out, which when it is the only one is the
  // task's title said back — `做每周总结页第一版` under `每周五自动出一页总结 view` — so a card
  // for it is a restatement wearing the cost of a node. **The one thing it did have to add is
  // now the row's own word**, `Working` (`runningHand`): `In progress` was read as saying it
  // and does not. Two hands are not a bigger version of that one word: which hands, how many,
  // and that one has stalled while another runs are all things the row cannot say. So nothing
  // is capped past that and nothing collapses into a number — four hands draw four, and a long
  // branch IS the news.
  for (const node of nodes.filter((n) => n.kind === "task" && n.data.sessions.length > 1)) {
    for (const session of node.data.sessions) {
      add({ id: session.id, kind: "activity", title: session.title,
        sourceRefs: [ref("session", session.id)],
        data: { session, taskId: node.id, ownerSessionId: session.ownerSessionId,
          currentAction: session.currentAction } });
      link(node.id, session.id, "works-on");
    }
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
  // The model now holds every closed row ever, so what makes one an update is the same fact
  // that used to make it a node at all: it closed and the person may not have seen it yet.
  for (const node of [...nodes]) {
    if (node.kind !== "task" || OPEN.has(node.data.status) || !node.data.inHand) continue;
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
    const kind = nodes.get(id)?.kind;
    // **Under a task: who is on it, then what it made.** A session is the live half of the
    // rank and a picture the finished half, and the two arrive in the order the projection
    // builds them, which is results first. Running work reads before its leftovers.
    if (kind === "task") { list.sort((a, b) => (a.kind === "activity" ? 0 : 1) - (b.kind === "activity" ? 0 : 1)); continue; }
    if (kind !== "group") continue;
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
 * on them says the opposite of what is true, and so does `Working`: a hand may well be in a
 * turn on some other part of the row, but the step the record is stopped on is theirs, and
 * that is the one thing the card exists to tell them.
 */
function waitsOnPerson(task) {
  return OPEN.has(task.status) && task.latest?.kind === "waiting";
}

/**
 * **One word, and every state it can take is a fact about the row.** A card used to be able to
 * carry a ledger status and a session state at once — `To do` above `Working`, `Completed`
 * above `Idle` — and most of that grid does not exist: nothing is being worked on while it is
 * still to do, and a closed row is closed whatever is still warm. What is left of the second
 * line is a single word the first line can hold.
 *
 * **One session fact reaches it: that a hand is in a turn, which the row then says as
 * `Working`** (`runningHand`). *In progress* was read as already saying that and never did.
 * A last turn that *failed* does not come with it: it used to replace the word, in the danger
 * tone, because it was the one thing a session knew that the ledger could not and the row had
 * nowhere else to say it — and every session on a task is a card below it now, wearing its own
 * word in its own tone. A cut turn is that card's word. The difference is that `Working` is a
 * fact about the row, true however many hands are on it, while a cut turn belongs to one of
 * them and says nothing about the others.
 *
 * A row waiting on the person is `needsYou` over everything — and that is a ledger fact too,
 * read from `latest`, not a session's.
 */
function stateOf(node) {
  if (node.kind === "task") {
    if (waitsOnPerson(node.data.task)) return "needsYou";
    return runningHand(node) ? "running" : node.data.status;
  }
  if (node.kind !== "activity") return node.kind;
  const s = node.data.session;
  if (s.state !== "running" && ["failed", "interrupted"].includes(s.lastTurn?.outcome)) return s.lastTurn.outcome;
  return s.state;
}

/**
 * **The hand whose turn the row is in**, or nothing. This is the one session fact that reaches
 * a task's status line, and it reaches it as the word — `Working` — not as a mark.
 *
 * `doing` is where the ledger row got to, not what is happening in it. On the record this was
 * written against, `shoe-comparison-master-20260918` had been `doing` for two days with no live
 * session at all and `game-screenshot-parse-poc-20260920` had a worker mid-turn, and the two
 * cards read the same *In progress* — the only difference on the chart was the age of the row's
 * status, which says nothing about whether anybody is on it. The one place *Working* appeared
 * was on a session's own card.
 *
 * **A mark was tried here and is the wrong instrument twice over.** This status line used to
 * carry a dot per hand, and `A mark is a poor way to say a thing a word can say` is why they
 * went; a single filled dot for *something is in flight* is the same mark making a smaller
 * claim, and a reader still has to be told what it means before the card says anything.
 *
 * Only an open row borrows it: a closed row is closed whatever is still warm beside it, and a
 * row waiting on the person keeps `needsYou`, which is the one word that asks them to act.
 * Where more than one hand is in a turn it is the one that has been in it longest — how long
 * this row has had something in flight, not when the latest hand happened to start.
 */
function runningHand(node) {
  if (node.kind !== "task" || !OPEN.has(node.data.status) || waitsOnPerson(node.data.task)) return null;
  const at = (s) => instant(s.stateSince) ?? Infinity;
  return node.data.sessions.filter((s) => s.state === "running").sort((a, b) => at(a) - at(b))[0] || null;
}

function age(value, now) {
  const at = instant(value);
  if (at === null) return L.unknown;
  const minutes = Math.max(0, Math.floor((now - at) / 60000));
  return minutes < 60 ? L.ago(minutes, "m") : minutes < 1440 ? L.ago(Math.floor(minutes / 60), "h") : L.ago(Math.floor(minutes / 1440), "d");
}

function nodeTime(node) {
  // A card's time is how long its status word has held, so a wait is timed from its line and a
  // turn from its own start: *Working · 3m ago* is a turn three minutes old, where the row's
  // own clock would have said how long it has been open and read as three days of work.
  if (node.kind === "task" && waitsOnPerson(node.data.task)) return node.data.task.latest.at || node.data.task.statusSince;
  const hand = runningHand(node);
  if (hand) return hand.stateSince || node.data.task.statusSince;
  return node.kind === "task" ? node.data.endedAt || node.data.task.statusSince
    : node.kind === "activity" ? node.data.session.stateSince
    : node.kind === "overview" ? node.data.updatedAt : null;
}

function emphasis(node, now) {
  const end = node.kind === "task" ? node.data.endedAt : null;
  if (instant(end) === null) return 1;
  // Fade emphasis, not legibility: even at its dimmest the card and its wire remain readable.
  return 1 - 0.22 * Math.min(1, Math.max(0, now - instant(end)) / FADE_MS);
}

/**
 * **A card is the size of a picture, and a picture is the size of its shot.** Shots are
 * rendered at a 16:9 frame and stored 960x540 (`view_shots.rs`), so 240x135 draws one with
 * nothing cropped and four image pixels to a CSS pixel — which is exactly 1:1 device pixels
 * at `ZOOM_MAX` on a retina screen. Stored 480x270 it was 1:1 at 1x and an upscale from
 * there, so the zoom that reaches in to look at a picture was the thing that blurred it.
 * A card is the same box because the two sit in one rank as peers of one another: two sizes
 * there made the rank read as two ranks.
 */
const CORE_W = 340, CORE_H = 320, CARD_W = 240, CARD_H = 135;
/**
 * The gutter between one rank and the next, **by the rank it leaves**. It is wider than it
 * needs to be to keep boxes apart, because it is also the room the wires bend in: a wire
 * leaves its parent's edge, runs to the midpoint and arrives flat at its child, so a narrow
 * gutter makes every curve the same near-vertical kink and the branch that owns a card stops
 * being readable from its wire.
 *
 * **The gutter off the core is the one that has to be widest, and it used to be the same 48px
 * as every other.** Every group on the chart parts from the core, so that one gutter carries
 * the whole fan — a dozen wires spreading over the full height of the drawing inside 48px of
 * run, which drew them as a near-vertical bundle leaving one edge rather than as a branch
 * each. Deeper ranks fan two or three ways over a card's height, and 48px is ample there.
 * Width is cheap on the widest axis only at the first rank: one gutter, paid once.
 *
 * Indexed by the depth of the *parent*, so `[0]` is the core's own.
 */
const RANK_GUTTER = [104, 56, 48];
const gutterAt = (depth) => RANK_GUTTER[Math.min(depth, RANK_GUTTER.length - 1)];
const MARGIN = 24;
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
 * **Home opens whole, and the scale that takes is chosen, not left to the day.** It opened fitted
 * once before, with a floor of 0.7, and lost: every open task was drawn, so the fit was whatever
 * the day's size forced and the floor was where every real day landed — a 17px title at 12px, a
 * picture at 168x95, to buy a view of the whole that nobody read at that size. It then opened at
 * 1x and scrolled, which kept the cards legible and put most of them outside the window. What
 * changed is that the chart is now cut to fit (`budgeted` at `OVERVIEW`), so the whole-chart
 * scale is a constant that was picked rather than a floor that was hit.
 *
 * Pinch and ⌘/Ctrl-wheel still reach in to read a picture and out past the whole.
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
 * and it lets any point of the drawing — its middle, where Home opens — be put at the centre
 * without a rule of its own.
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
 * Semantic edges determine the hierarchy; flextree only computes its geometry.
 * Overview children are embedded INSIDE the core, not duplicated as peripheral cards.
 *
 * It draws the model it is given. What that is — which cards the window has room for — is
 * `budgeted`'s call, made before this runs.
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
    // `n.depth` is the rank the gutter leaves: the hub root is the core, so its gutter is the
    // one the whole fan bends in. A gutter is a node's own padding here, not a pair's, which
    // is right — every child of one parent leaves through the same run.
    const layout = flextree({ nodeSize: (n) => [n.data.h, n.data.w + gutterAt(n.depth)],
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
  const wires = model.edges.filter((e) => e.primary && byId.has(e.from) && byId.has(e.to)).map((edge) =>
    ({ ...edge, paint: branchPaint(tones.get(edge.to)), d: wirePath(byId.get(edge.from), byId.get(edge.to)) }));
  return { placed, wires, width, height };
}

/**
 * A wire leaves its parent's edge, runs to the midpoint and arrives flat at its child. Any two
 * boxes will do, which is what lets the glide draw one between two cards that are mid-flight.
 */
function wirePath(from, to) {
  const right = to.dir > 0;
  const sx = from.x + (right ? from.w : 0), sy = from.y + from.h / 2;
  const ex = to.x + (right ? 0 : to.w), ey = to.y + to.h / 2, mid = (sx + ex) / 2;
  return `M ${sx} ${sy} C ${mid} ${sy}, ${mid} ${ey}, ${ex} ${ey}`;
}

/**
 * **The chart opens whole, at one scale, so it holds what fits at that scale.** An ordinary day
 * drew 22 cards over 2308x2178, and seeing all of it in a 1512x855 window took 0.39 — a 17px
 * title at 7px. At 0.8 the same window holds 12 of them, two pictures and every group.
 *
 * The two axes are bought differently, and that is the whole rule. **Width is bought by depth**:
 * a chart is exactly as wide as its deepest path, whatever the day holds. **Height is bought by
 * cards**: every one stacks. So nothing that makes depth is cut — every group is drawn where it
 * is, and one whose cards are all put away is its label and a count, 56px tall, still holding
 * its rank. Only cards are cut, and only until the drawing is the window's shape at this scale.
 * The first try cut depth instead, drawing first-level groups alone: it narrowed the chart to
 * 1348, left the width it was meant to use empty, and hung inner groups' cards off their parent,
 * which is a different tree.
 *
 * 0.8 is where the pictures start to fit: at 0.87 the width goes to inner groups and no picture
 * is drawn, and at 0.7 a title is back to the 12px that fitting was rejected for before.
 */
const OVERVIEW = 0.8;
const CANDIDATES = 64;

/**
 * How much a card is in hand: waiting on the person first — the one status that asks them to
 * act — then anybody running on it, then how lately it moved.
 */
/**
 * How much a card wants the window, as a tier and then a time.
 *
 * **The tier is what keeps a complete model from drawing like an archive.** Every row the
 * ledger has is a node now, so "how lately it moved" alone would let a report that closed an
 * hour ago outrank a to-do nobody has touched in a week — the thing that is actually in hand.
 * Waiting on the person is still first, because it is the one status that asks them to act.
 *
 *   4 waiting on the person · 3 somebody running on it · 2 open · 1 closed and in hand · 0 history
 *
 * Within a tier, how lately it moved. Heat decides whether a card is on the chart and never
 * where: what is drawn keeps the record's order, so an update moves only the cards it changes.
 */
function heat(node) {
  const task = node.data.task;
  const sessions = node.kind === "activity" ? [node.data.session] : node.data.sessions || [];
  const open = node.kind === "activity" || (!!task && OPEN.has(task.status));
  const tier = task && waitsOnPerson(task) ? 4
    : sessions.some((s) => s.state === "running") ? 3
    : open ? 2 : node.data.inHand ? 1 : 0;
  return [tier,
    Math.max(instant(task?.latest?.at) ?? 0, instant(task?.statusSince) ?? 0,
      ...sessions.map((s) => instant(s.stateSince) ?? 0))];
}
/** A card that is part of what is going on now, rather than what a branch has been through. */
const inHand = (node) => heat(node)[0] >= 1;

/**
 * **The model is the whole graph; this picks what the window draws of it**, in `frame` at
 * `OVERVIEW`, and counts what a group holds in hand that did not fit.
 *
 * Nothing but a spent cancellation is kept out of the model (`onHome`), so this is the only
 * place any other card is ever left off the chart. Cards are offered hottest first — the tier
 * in `heat`, so every card in hand is offered before any history — and each is tried against
 * the whole chart laid out afresh:
 *
 * 1. **In a group taken as the centre, everything it holds in hand is drawn**, at any depth
 *    below it, inner groups included: the person pressed in to see this thread. It is the one
 *    thing here that may overflow the window, which is what the overview scale and a scroll
 *    are for. On the whole chart the core's own cards — ungrouped work — get no such pass.
 *    Exempting them was watched failing: a render with no transcript held nineteen closed,
 *    ungrouped notices, they took the whole window, and every group was left a bare label. And
 *    for someone who has never grouped anything, every card is ungrouped, so nothing would ever
 *    be cut. What the core puts away it counts, and that count opens the task board.
 * 2. **Every branch with work in hand draws its hottest card that fits**, so no branch reads as
 *    empty when it is only quiet. An ungrouped card is a branch of its own. A branch holding
 *    nothing but history gets no such floor — it is drawn if there is room and not otherwise.
 * 3. **Then the rest, hottest first, each while the whole still fits.** A card that would not is
 *    passed over and the next is tried, so the shorter side fills. **History is the tail of this
 *    pass**: a branch's finished work is drawn in whatever room its thread's live work leaves,
 *    which on a full day is none and in a group taken as the centre is most of the window.
 * 4. **Pictures last, in what width is left.** A picture spends a rank of width and the width is
 *    both sides' at once, so offered with its card, one picture on the right kept a card still in
 *    progress out of its inner group on the left, and the branch showed one closed eight hours before.
 *
 * What is drawn keeps the order the record gives it: heat decides whether a card is on the chart,
 * never where, so an update moves only the cards it changes.
 *
 * **Only in-hand cards are counted.** *3 more* is an invitation to press in; "and 47 things
 * that finished" is not one, and the number would be the ledger's depth rather than anything
 * about the work. History that did not fit is simply not drawn.
 */
function budgeted(model, frame, tones) {
  const children = childIndex(model);
  const parent = new Map(model.edges.filter((e) => e.primary).map((e) => [e.to, e.from]));
  const room = { w: frame.w / OVERVIEW, h: frame.h / OVERVIEW };
  const all = model.nodes.filter((n) => n.kind === "task" || n.kind === "activity")
    .sort((a, b) => { const x = heat(a), y = heat(b); return y[0] - x[0] || y[1] - x[1]; });
  // **What the fitting loop costs is bounded by the window, not by the ledger.** Every offer
  // lays the whole chart out again, and a complete model can hold hundreds of rows against a
  // window that draws a dozen. Everything in hand is always a candidate — the sort puts it
  // first — and history is offered `CANDIDATES` deep, which is several times what any window
  // has ever drawn.
  const cards = all.filter((n, i) => inHand(n) || i < CANDIDATES);
  const shown = new Set();
  const groupsAbove = (id) => {
    const out = [];
    for (let at = parent.get(id); at && at !== model.rootId; at = parent.get(at)) out.push(at);
    return out;
  };
  // **A group is drawn on the way to a card, or because it holds work in hand.** The second
  // half is what keeps a busy group that lost the fit from vanishing: it is a label and its
  // count, still holding its rank. The first is what draws a finished group when a branch has
  // the room for its history. A group with neither is not a node — structure standing where
  // its content used to be is what this surface refuses.
  const standing = new Set();
  for (const card of all) if (inHand(card)) for (const id of groupsAbove(card.id)) standing.add(id);
  const cut = () => {
    const reached = new Set();
    for (const id of shown) for (const above of groupsAbove(id)) reached.add(above);
    const keep = (n) => n.id === model.rootId || n.kind === "overview" || shown.has(n.id)
      || (n.kind === "group" && (standing.has(n.id) || reached.has(n.id)));
    const ids = new Set(model.nodes.filter(keep).map((n) => n.id));
    return { ...model, nodes: model.nodes.filter((n) => ids.has(n.id)),
      edges: model.edges.filter((e) => ids.has(e.from) && ids.has(e.to)) };
  };
  const offer = (ids) => {
    ids.forEach((id) => shown.add(id));
    const drawn = arrange(cut(), tones);
    if (drawn.width <= room.w && drawn.height <= room.h) return true;
    ids.forEach((id) => shown.delete(id));
    return false;
  };
  const branchOf = (id) => { let at = id; while (parent.get(at) !== model.rootId) at = parent.get(at); return at; };
  const pressedInto = model.nodes.find((n) => n.id === model.rootId)?.kind === "group";
  if (pressedInto) for (const card of cards) if (inHand(card)) shown.add(card.id);
  const floored = new Set();
  for (const card of cards) {
    const branch = branchOf(card.id);
    if (!shown.has(card.id) && inHand(card) && !floored.has(branch) && offer([card.id])) floored.add(branch);
  }
  for (const card of cards) if (!shown.has(card.id)) offer([card.id]);
  for (const card of cards) {
    const tiles = shown.has(card.id) ? (children.get(card.id) || []).filter((k) => k.kind === "result").map((k) => k.id) : [];
    if (tiles.length && !offer(tiles)) offer(tiles.slice(0, 1));
  }
  const hidden = new Map();
  for (const card of all) {
    if (shown.has(card.id) || !inHand(card)) continue;
    hidden.set(parent.get(card.id), (hidden.get(parent.get(card.id)) || 0) + 1);
  }
  return { model: cut(), hidden };
}

/**
 * The scale the window opens at: the whole drawing, never above 1x — and **never below
 * `OVERVIEW`**. The cut makes the whole fit at that scale except when what it may not cut does
 * not: every group's label, or a group's own cards once it is the centre. Then the window opens
 * at `OVERVIEW` and scrolls, which is legible, rather than shrinking to whatever the day forces,
 * which is the floor fitting was rejected for.
 */
function opening(chart, frame) {
  return clampZoom(Math.max(OVERVIEW, Math.min(frame.w / chart.width, frame.h / chart.height, 1)));
}

/**
 * **An update moves the chart; it does not redraw it.** A card that leaves fades out where it
 * stood, a card that stays glides from where it was on screen to where it now is, and a card
 * that arrives fades in after them, into the room the other two made.
 *
 * An update used to be a redraw, and it read as a flash. Measured on a live arrangement by
 * dropping one group's seven tasks from the ledger: the group's five headings went in a single
 * frame, every one of the fifteen that stayed jumped to its new place in the next — one 956px,
 * across the core, because the balance moved it to the other side — and **nine of those fifteen
 * faded to nothing and back in.** That last one was not a choice anybody made: the rows were
 * drawn in layout order, a row that changed places in it was re-inserted, and a re-inserted
 * element starts its entry animation over. So what is drawn is now in id order, which no update
 * changes, and nothing that stays is ever re-inserted.
 *
 * **A glide is from the screen, not from the chart.** The same card sits at different chart
 * coordinates in two drawings even when it has not moved, because a drawing is measured from
 * its own top-left, and a refit changes the scale under all of it at once (`opening`). So where
 * a card starts is where it *was on screen*, carried into the new drawing's units (`carry`),
 * and the scale change is part of the same glide instead of a jump in front of it.
 *
 * **A card that changes sides does not travel; it hops.** The balance can move a whole branch
 * to the other side of the core (`arrange`), and glided, every card of it slid across the core
 * card and through the branches it passed, trailing a wire drawn edge to edge through its own
 * label. So a card whose side changed fades out where it was, changes sides while it cannot be
 * seen, and fades in where it now is — a branch leaving one side and arriving on the other,
 * which is what happened. Its wires fade with it.
 *
 * Only an update the person was looking at glides. The first drawing, the one the sources
 * answer into, a resize, and the narrow flow are drawn the way they always were, and a reader
 * who asked for reduced motion gets none of this.
 */
const GLIDE_MS = 420, EXIT_MS = 220;
/** A hop's opacity over the glide: out, a moment of nothing while it changes sides, back in. */
const HOP_FADE = [{ opacity: 0, offset: 0.4 }, { opacity: 0, offset: 0.6 }];
/** An arriving card waits most of the way through the leaving ones' fade before it starts its own. */
const ENTER_AFTER = 160;
/** Cards that arrive together come in one after another: 28ms apart, the eighth and later together. */
const STAGGER_MS = 28, STAGGER_CAP = 8;
const ease = (t) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2);
const lerpBox = (a, b, e) => ({ x: a.x + (b.x - a.x) * e, y: a.y + (b.y - a.y) * e,
  w: a.w + (b.w - a.w) * e, h: a.h + (b.h - a.h) * e, dir: b.dir });
const sameBox = (a, b) => Math.abs(a.x - b.x) < 0.5 && Math.abs(a.y - b.y) < 0.5
  && Math.abs(a.w - b.w) < 0.5 && Math.abs(a.h - b.h) < 0.5;
/** Where a move is at eased progress `e`. A hop is on its old side until halfway, then its new. */
const boxAt = (move, e) => (move.hop ? (e < 0.5 ? move.from : move.to) : lerpBox(move.from, move.to, e));

/**
 * The box on stage `to` that sits exactly where `box` sat on stage `from`. A stage is what puts
 * chart units on screen: `{ scale, offset, scroll }`, the same three `stage` and the viewport
 * already hold.
 */
function carry(box, from, to) {
  const k = from.scale / to.scale;
  return {
    x: (from.offset.x + box.x * from.scale - from.scroll.left + to.scroll.left - to.offset.x) / to.scale,
    y: (from.offset.y + box.y * from.scale - from.scroll.top + to.scroll.top - to.offset.y) / to.scale,
    w: box.w * k, h: box.h * k, dir: box.dir,
  };
}

/**
 * Where an element rendered at `rendered` is drawn when it is made to stand for `box`: scaled
 * alike on both axes, about the box's centre. A card that glides keeps its shape; only a group
 * taken as the centre changes it, and there squashing its label would be the one thing wrong.
 * `k` is the scale, and the wires run between these boxes rather than the targets.
 */
function drawn(rendered, box) {
  const k = Math.sqrt((box.w / rendered.w) * (box.h / rendered.h));
  const w = rendered.w * k, h = rendered.h * k;
  return { x: box.x + (box.w - w) / 2, y: box.y + (box.h - h) / 2, w, h, k, dir: box.dir };
}

/**
 * What an update does on screen. `was` is every box on screen just before it, by node id, as
 * `{ row, box }`: the row the element is rendered at and the box it is drawn at, in the
 * previous drawing's units. Returns the new drawing's units throughout:
 *
 * - `moves`: a node in both drawings whose box changed on screen, `{ from, to, hop }` — `to` is
 *   its new row, so a node that stayed exactly where it was is not a move at all, and `hop` is
 *   whether it changed sides of the core.
 * - `leaving`: a node the new drawing does not have, `{ row, box }` — still rendered at its old
 *   row and drawn where it was on screen, for as long as it takes to fade.
 *
 * A node only in the new drawing is in neither: it arrives, which is the entry animation's.
 */
function glide(was, before, after, chart) {
  const moves = new Map(), leaving = new Map(), ids = new Set();
  for (const row of chart.placed) {
    ids.add(row.node.id);
    const prior = was.get(row.node.id);
    if (!prior) continue;
    const from = carry(prior.box, before, after);
    const hop = !!prior.row.dir && !!row.dir && prior.row.dir !== row.dir;
    if (!sameBox(from, row)) moves.set(row.node.id, { from, to: row, hop });
  }
  for (const [id, prior] of was) if (!ids.has(id)) leaving.set(id, { row: prior.row, box: carry(prior.box, before, after) });
  return { moves, leaving };
}

async function getJson(path) {
  const response = await fetch(path);
  if (!response.ok) throw new Error(`${response.status}`);
  return response.json();
}

/**
 * **The group this window took as the centre is kept by the window, for the next time Home is
 * opened in it.** It is not a record and nothing else reads it: another device, or a phone beside
 * this laptop, keeps its own. A store that refuses (a private window, storage switched off) opens
 * Home the way a first visit does.
 *
 * It used to keep the scale and the card in the middle too, because the chart was larger than
 * the window and there was a place in it to lose. The chart now opens whole, so there is not;
 * the key is written whole, so a window's old `scale` and `anchor` go the next time it takes a
 * centre.
 */
const PLACE_KEY = "hi-home-place";
function readPlace() {
  try {
    const place = JSON.parse(localStorage.getItem(PLACE_KEY) || "null");
    return place && typeof place === "object" ? place : {};
  } catch { return {}; }
}
function writePlace(place) {
  try { localStorage.setItem(PLACE_KEY, JSON.stringify(place)); } catch { /* opens fresh next time */ }
}

export default function Home() {
  // Not `trail`: that name is the path back out of a centred group, and shadowing it here made
  // every window with a kept centre throw on its first render and leave the stage empty.
  const { openRef, trail: viewTrail } = useViews();
  const { messages } = useMessages();
  // What was put up while they were on another page and has not been opened since — the
  // screen's list says so, and a task that made one wears a dot until it is opened.
  const unopened = useMemo(
    () => new Set((viewTrail || []).filter((entry) => entry.unopened && entry.view_ref).map((entry) => entry.view_ref)),
    [viewTrail],
  );
  const [source, setSource] = useState({ tasks: [], workers: [], groups: [] });
  const [loaded, setLoaded] = useState(false);
  const [ledgerSettled, setLedgerSettled] = useState(false);
  const [sourcesSettled, setSourcesSettled] = useState(false);
  const [frameMeasured, setFrameMeasured] = useState(false);
  const [errors, setErrors] = useState([]);
  const [now, setNow] = useState(Date.now);
  const [frame, setFrame] = useState({ w: 1200, h: 760 });
  const viewport = useRef(null), stageRef = useRef(null), inFlight = useRef(false);
  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      // `/api/workers/ended` is gone from this view: an ended session is not in hand, and
      // `factory/workers` is the surface that owns session history.
      const requests = [
        ["workers", "/api/workers", (v) => v.workers],
        // No `/api/views`: it was read every tick only to find each `made` ref's shot, and the
        // ledger's rows carry what they placed now, pictures and all (`attached`).
        //
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
  // The open panel is held by subject, and its row read from this surface's own ledger on each
  // render, so a status moved in the panel reaches the card and the panel by the one read. A row
  // that leaves the ledger closes it.
  const [openSubject, setOpenSubject] = useState(null);
  // Which line the panel opens at, and which of the things it carries is opened whole — set by
  // an attachment's tile, absent for a card.
  const [openFocus, setOpenFocus] = useState(null);
  const [TaskPanel, setTaskPanel] = useState(null);
  // Fetched before anybody presses, so a press does not wait on a compile and a module.
  useEffect(() => { loadTaskPanel().then((panel) => setTaskPanel(() => panel), () => {}); }, []);
  const openTask = useCallback((subject, focus = null) => {
    loadTaskPanel().then((panel) => { setTaskPanel(() => panel); setOpenFocus(focus); setOpenSubject(subject); },
      (error) => console.warn("the task panel did not load", error));
  }, []);
  const closeTask = useCallback(() => { setOpenSubject(null); setOpenFocus(null); }, []);
  const openRow = openSubject ? source.tasks.find((task) => task.subject === openSubject) : null;
  const model = useMemo(() => buildHome({ ...source, messages }, now), [source, messages, now]);
  const children = useMemo(() => childIndex(model), [model]);
  const tones = useMemo(() => branchTones(model), [model]);
  const kept = useRef(null);
  if (kept.current === null) kept.current = readPlace();
  const [focus, setFocus] = useState(() => (typeof kept.current.focus === "string" ? kept.current.focus : null));
  const focused = useMemo(() => focusOn(model, focus), [model, focus]);
  const shown = focused || model;
  const mobile = frame.w < 760;
  // The narrow flow is a list the page scrolls, so only the chart has a window to fill.
  const overview = useMemo(() => (mobile ? { model: shown, hidden: new Map() } : budgeted(shown, frame, tones)),
    [shown, frame, tones, mobile]);
  const chart = useMemo(() => arrange(overview.model, tones), [overview, tones]);
  const path = useMemo(() => (focused ? trail(model, focus) : []), [model, focused, focus]);
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
  // **The chart opens whole, and stays whole until the person takes the window.** `fit` is the
  // scale `opening` picks; a zoom or a pan is theirs from then on, until they take another
  // centre — a new chart, which opens whole again.
  const fit = opening(chart, frame);
  const [held, setHeld] = useState(null);
  const scale = held ?? fit;
  const { canvas, offset } = stage(chart, frame, scale);
  // Wheel and gesture events arrive faster than renders, so each zoom composes on the scale and
  // scroll the previous one asked for rather than on what is on screen yet.
  const live = useRef({ scale, chart, frame, focus }), pending = useRef(null);
  live.current = { scale: pending.current?.scale ?? scale, chart, frame, focus };
  const hold = useCallback(() => setHeld((was) => was ?? live.current.scale), []);
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
    setHeld(to);
  }, []);
  useLayoutEffect(() => {
    if (!pending.current || !viewport.current) return;
    viewport.current.scrollTo({ left: pending.current.left, top: pending.current.top, behavior: "instant" });
    pending.current = null;
  }, [scale]);
  // While the window is whole, the drawing sits in its middle: on the first open, when the sources
  // answer, when the window is resized, when a card comes or goes. **The drawing's middle, not the
  // core's** — the two sides are rarely the same height, and centring the core put the taller
  // side's last card half below the window of a chart that fit it.
  useLayoutEffect(() => {
    if (held !== null || mobile || !frameMeasured || !viewport.current) return;
    scrollToPoint({ x: chart.width / 2, y: chart.height / 2 });
  }, [held, mobile, frameMeasured, chart, frame, scale, offset.x, offset.y]);
  // After the centring above, never before it: a glide starts from where a card was on screen,
  // and needs the scroll this drawing ends up at to say where that is now.
  const still = typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  const glides = useGlide({ chart, scale, offset, frame, viewport, stage: stageRef,
    ready: loaded && frameMeasured && !mobile && !still });
  // Another centre is another chart, and it opens whole.
  const centreOn = useCallback((next) => {
    if (next === live.current.focus) return;
    setFocus(next);
    setHeld(null);
    writePlace({ focus: next });
  }, []);
  const pointers = useRef(new Map()), gesture = useRef(null), dragged = useRef(false);
  // Pinch and ⌘/Ctrl-wheel zoom at the pointer. A plain wheel still scrolls. These are
  // registered by hand because React's wheel listener is passive and cannot stop the page
  // itself from zooming; `gesture*` is how Safari and WKWebView report a trackpad pinch.
  useEffect(() => {
    const el = viewport.current;
    if (!el || mobile) return;
    const at = (e) => { const r = el.getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; };
    const wheel = (e) => {
      // A plain wheel scrolls, and a scroll is the person moving the window: from here an update
      // must not pull it back to the middle.
      if (!e.ctrlKey && !e.metaKey) { hold(); return; }
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
  }, [mobile, zoomTo, hold]);
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
  const common = { now, children, openRef, openTask, tones, centreOn, unopened };
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
        onScroll={glides.onScroll} onPointerDown={(e) => {
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
          hold();
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
          <div className="hi-work__stage" ref={stageRef} style={{ left: offset.x, top: offset.y, width: chart.width, height: chart.height,
            transform: `scale(${scale})` }}>
            <svg className="hi-work__wires" width={chart.width} height={chart.height} aria-hidden>
              {glides.wires.map(({ wire, leaving, delay }) => <path key={wire.id} data-edge={wire.id}
                data-leaving={leaving ? "" : undefined} d={wire.d}
                style={{ stroke: wire.paint, "--enter-delay": `${delay}ms` }} />)}
            </svg>
            {/* The side a node sits on is on the node, because a group's icon takes the end
                of its heading that faces the core — see `.hi-work__group > button` in the CSS.
                A row on its way out is still a row, so its card keeps its element and its
                picture while it fades; it just cannot be pressed. */}
            {glides.rows.map(({ row, leaving, delay }) => <div key={row.node.id} className="hi-work__position"
              data-row={row.node.id} data-dir={row.dir} data-leaving={leaving ? "" : undefined} inert={leaving}
              style={{ left: row.x, top: row.y, width: row.w, height: row.h,
                "--enter-delay": `${delay}ms` }}>
              {row.node.kind === "core" ? <Core node={model.nodes[0]} model={model} now={now}
                more={overview.hidden.get(row.node.id)} openRef={openRef} />
                : <Node node={row.node} root={row.node.id === shown.rootId} up={up}
                  more={overview.hidden.get(row.node.id)} {...common} />}
            </div>)}
          </div>
        </div>}
      </div>
      </div>
      {/* Over the frame, not in it: the chart's pan and pinch are the viewport's, and a press in
          the panel is not one of them. A roster that has not landed is not an empty one. */}
      {openRow && TaskPanel && <TaskPanel task={openRow} onClose={closeTask} focus={openFocus}
        workers={sourcesSettled && !errors.includes("workers") ? source.workers : undefined} />}
    </div>
  );
}

/**
 * **A task opens the board's own panel, drawn here — not a copy of it.** `factory/tasks` exports
 * it (`TaskPanel`), and a view is compiled one file at a time with only its bare imports kept, so
 * it is reached the way the host mounts any view: ask the core for the compiled module
 * (`GET /api/views/module`, which moves nothing) and `import()` it. Asked once per page; a
 * failure is forgotten so the next press asks again.
 */
let taskPanel = null;
function loadTaskPanel() {
  taskPanel ??= getJson(`/api/views/module?ref=${encodeURIComponent(TASK_BOARD)}`)
    .then(({ module_url }) => import(module_url))
    .then(({ TaskPanel }) => {
      if (typeof TaskPanel !== "function") throw new Error(`${TASK_BOARD} exports no TaskPanel`);
      return TaskPanel;
    })
    .catch((error) => { taskPanel = null; throw error; });
  return taskPanel;
}

/**
 * The chart's motion from one drawing to the next — `GLIDE_MS` says what it is and why.
 *
 * It returns what to render: the drawing's rows and wires plus whatever is still on its way out,
 * in id order, each with the delay its entry animation was given when it first appeared.
 * Everything after that is the element's own: a transform on the row and a path on the wire,
 * written straight onto the DOM for the length of the glide and taken off again at the end.
 * **Layout stays the drawing's**: `left`/`top` are always the new rows, the stage's zoom is
 * never touched, and a glide cut short by the next update starts that one from wherever the
 * cards had got to.
 *
 * `ready` is whether this drawing is one the person has been looking at.
 */
function useGlide({ chart, scale, offset, frame, viewport, stage, ready }) {
  const ref = useRef(null);
  if (ref.current === null) ref.current = { chart: null, stage: null, scroll: null, frame: null, ready: false,
    moves: new Map(), started: 0, leaving: new Map(), wires: new Map(), delays: new Map(), hops: [], raf: 0 };
  const s = ref.current;
  const [, settle] = useState(0);
  const same = s.chart === chart;
  const glides = ready && s.ready && !!s.chart && !same && s.frame?.w === frame.w && s.frame?.h === frame.h;
  // What stays on screen past this drawing: whatever was already on its way out and, when this
  // update glides, whatever the last drawing had and this one does not. Anything else drops.
  const ids = new Set(chart.placed.map((row) => row.node.id));
  const going = new Map();
  if (same || glides) for (const [id, l] of s.leaving) if (!ids.has(id)) going.set(id, l.row);
  if (glides) for (const row of s.chart.placed) if (!ids.has(row.node.id) && !going.has(row.node.id)) going.set(row.node.id, row);
  const shown = new Set([...ids, ...going.keys()]);
  const edges = new Set(chart.wires.map((w) => w.id));
  const fading = new Map();
  if (same || glides) for (const [id, w] of s.wires) if (!edges.has(id)) fading.set(id, w);
  if (glides) for (const w of s.chart.wires) if (!edges.has(w.id) && !fading.has(w.id)) fading.set(w.id, w);
  for (const [id, w] of fading) if (!shown.has(w.from) || !shown.has(w.to)) fading.delete(id);
  // A row's entry delay is fixed when it first appears. Changing it later would move an entry
  // that is still waiting to start.
  const fresh = new Map();
  let k = 0;
  for (const row of chart.placed) {
    if (s.delays.has(row.node.id)) continue;
    fresh.set(row.node.id, (glides ? ENTER_AFTER : 0) + Math.min(k++, STAGGER_CAP) * STAGGER_MS);
  }
  const delay = (id) => s.delays.get(id) ?? fresh.get(id) ?? 0;
  const byId = (a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  const rows = [...chart.placed.map((row) => ({ id: row.node.id, row, leaving: false })),
    ...[...going].map(([id, row]) => ({ id, row, leaving: true }))].sort(byId)
    .map((r) => ({ ...r, delay: delay(r.id) }));
  const wires = [...chart.wires.map((wire) => ({ id: wire.id, wire, leaving: false })),
    ...[...fading].map(([id, wire]) => ({ id, wire, leaving: true }))].sort(byId)
    .map((w) => ({ ...w, delay: w.leaving ? 0 : delay(w.wire.to) }));

  // One frame of the glide: every row and wire as it stands at this instant, and the next frame
  // asked for while anything is still in flight.
  const paint = useCallback(() => {
    const s = ref.current, root = stage.current;
    cancelAnimationFrame(s.raf);
    s.raf = 0;
    if (!root || !s.chart) return;
    const now = performance.now();
    const e = s.moves.size ? ease(Math.min(1, (now - s.started) / GLIDE_MS)) : 1;
    const els = new Map([...root.querySelectorAll("[data-row]")].map((el) => [el.dataset.row, el]));
    const at = new Map(s.chart.placed.map((row) => [row.node.id, row]));
    const place = (id, rendered, box) => {
      const d = drawn(rendered, box), el = els.get(id);
      at.set(id, d);
      if (el) el.style.transform = `translate(${d.x - rendered.x}px, ${d.y - rendered.y}px) scale(${d.k})`;
    };
    for (const [id, m] of s.moves) place(id, m.to, boxAt(m, e));
    for (const [id, l] of s.leaving) place(id, l.row, l.box);
    const finals = new Map(s.chart.wires.map((w) => [w.id, w]));
    const done = s.moves.size > 0 && e >= 1;
    if (s.moves.size || s.wires.size) {
      for (const path of root.querySelectorAll("path[data-edge]")) {
        const wire = finals.get(path.dataset.edge) || s.wires.get(path.dataset.edge);
        const from = wire && at.get(wire.from), to = wire && at.get(wire.to);
        if (from && to) path.setAttribute("d", done && finals.has(wire.id) ? wire.d : wirePath(from, to));
      }
    }
    if (done) {
      for (const id of s.moves.keys()) if (els.get(id)) els.get(id).style.transform = "";
      s.moves = new Map();
    }
    let gone = false;
    for (const [id, l] of s.leaving) if (now - l.since >= EXIT_MS) { s.leaving.delete(id); gone = true; }
    for (const [id, w] of s.wires) if (now - w.since >= EXIT_MS) { s.wires.delete(id); gone = true; }
    if (gone) settle((n) => n + 1);
    if (s.moves.size || s.leaving.size || s.wires.size) s.raf = requestAnimationFrame(paint);
  }, [stage]);

  useLayoutEffect(() => {
    const s = ref.current, el = viewport.current;
    const scroll = el ? { left: el.scrollLeft, top: el.scrollTop } : { left: 0, top: 0 };
    if (!same) {
      const now = performance.now(), root = stage.current;
      // A hop cut short is at whatever opacity it had got to; the next update starts from full.
      for (const fade of s.hops) fade.cancel();
      s.hops = [];
      if (glides) {
        // Where everything was on screen the instant before this drawing — mid-glide or not.
        const e = s.moves.size ? ease(Math.min(1, (now - s.started) / GLIDE_MS)) : 1;
        const was = new Map();
        for (const row of s.chart.placed) {
          const m = s.moves.get(row.node.id);
          was.set(row.node.id, { row, box: m ? drawn(row, boxAt(m, e)) : row });
        }
        for (const [id, l] of s.leaving) if (!was.has(id)) was.set(id, l);
        const plan = glide(was, { ...s.stage, scroll: s.scroll }, { scale, offset, scroll }, chart);
        for (const [id, l] of plan.leaving) l.since = s.leaving.get(id)?.since ?? now;
        s.moves = plan.moves; s.started = now; s.leaving = plan.leaving;
        s.wires = new Map([...fading].map(([id, w]) => [id, { ...w, since: s.wires.get(id)?.since ?? now }]));
        // The fade is the card's own pane, for the reason the exit's is (see the CSS), and every
        // wire with a hopping end goes with it. Its ends are left implicit, so it dips from and
        // back to whatever the card's emphasis is.
        const hops = new Set([...plan.moves].filter(([, m]) => m.hop).map(([id]) => id));
        if (root && hops.size) {
          for (const el of root.querySelectorAll("[data-row]")) {
            if (hops.has(el.dataset.row) && el.firstElementChild) s.hops.push(el.firstElementChild.animate(HOP_FADE, GLIDE_MS));
          }
          const ends = new Map([...chart.wires, ...fading.values()].map((w) => [w.id, w]));
          for (const path of root.querySelectorAll("path[data-edge]")) {
            const w = ends.get(path.dataset.edge);
            if (w && (hops.has(w.from) || hops.has(w.to))) s.hops.push(path.animate(HOP_FADE, GLIDE_MS));
          }
        }
      } else {
        // Not an update anyone was watching: the new drawing, exactly as it lays out.
        s.moves = new Map(); s.leaving = new Map(); s.wires = new Map();
        if (root) {
          for (const row of root.querySelectorAll("[data-row]")) row.style.transform = "";
          const finals = new Map(chart.wires.map((w) => [w.id, w.d]));
          for (const path of root.querySelectorAll("path[data-edge]")) if (finals.has(path.dataset.edge)) path.setAttribute("d", finals.get(path.dataset.edge));
        }
      }
      s.chart = chart;
    }
    s.stage = { scale, offset }; s.scroll = scroll; s.frame = frame; s.ready = ready;
    for (const [id, d] of fresh) s.delays.set(id, d);
    for (const id of s.delays.keys()) if (!shown.has(id)) s.delays.delete(id);
    // Written before the browser paints, so no frame ever shows the new drawing un-glided.
    paint();
  });
  useEffect(() => () => cancelAnimationFrame(ref.current.raf), []);
  // The scroll the next update starts from. Every commit records it once the centring has moved
  // it; this is what keeps it true while the person scrolls in between.
  const onScroll = useCallback((event) => {
    ref.current.scroll = { left: event.currentTarget.scrollLeft, top: event.currentTarget.scrollTop };
  }, []);
  return { rows, wires, onScroll };
}

/**
 * Read-only, but for one handoff. Every affordance this card had opened a detail dialog that no
 * longer exists. The one it has is the count of ungrouped work the chart put away (`budgeted`):
 * that work has no group to press into, so the count opens the board that carries all of it —
 * with the same named loan every card's handoff has, the board and not the rows.
 */
function Core({ node, model, now, more = 0, openRef }) {
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
    {more > 0 && <button className="hi-work__core-more" onClick={() => openRef(TASK_BOARD)}>{L.onBoard(more)}</button>}
  </article>;
}

/**
 * A card is a glance, and its whole surface is one handoff.
 *
 * **What a card is, it says without a label.** There are two kinds of card — a task and a live
 * session working on one — and a word naming the kind was the first line of every card. A
 * session instead wears the dot the core's roles wear, because it is the same fact: the dot
 * means a live session, filled while it is running. A task has none: what a hand on it puts on
 * the row is a word, `Working` (`runningHand`), because this line threw its marks out. The
 * status word carries the tone a coloured left edge used to, so no side of the border means
 * anything.
 *
 * **A task opens its own panel over this surface** — the board's, through `openTask`, so the
 * row is read, answered and moved without leaving Home.
 *
 * **Named loan:** a session opens its board, not its row on it. `openRef(viewRef)` carries a
 * view ref and nothing else, and `factory/workers` reads no incoming target and exports no
 * panel, so you arrive at the board and find the session yourself. What takes this back is
 * the same move the task made — `factory/workers` exporting its detail for this surface to
 * draw; see `docs/arch/home.md` § Open.
 */
function Node({ node, now, children, openRef, openTask, tones, centreOn, unopened = new Set(), root = false, up = null, more = 0 }) {
  const state = stateOf(node), time = nodeTime(node);
  // A dot for something this task made that was put up while they were on another page and
  // is still waiting unopened in their list. Opening it — here, from the tile — clears it.
  const fresh = node.kind === "result" ? unopened.has(node.data.viewRef)
    : node.kind === "task" && (node.data.results || []).some((result) => result.viewRef && unopened.has(result.viewRef));
  const open = node.kind === "task" ? () => openTask(node.data.task.subject)
    : node.kind === "activity" ? () => openRef(SESSION_BOARD) : null;
  // A tile lands exactly where it points: `openRef` takes a view ref natively, so it needs none
  // of the targeting the session handoff is waiting on.
  // **A tile opens what it is a picture of.** A view is gone to, as it always was. A picture or
  // a clip opens its task's panel at the line that carried it, and the panel's viewer draws it
  // whole with that line as its caption — the words and the evidence arrive together, which is
  // the point of a line carrying one (`docs/arch/home.md` § Handing off).
  if (node.kind === "result") return <article className="hi-work__tile" data-node-id={node.id}
    data-kind="result" data-shows={node.data.kind}>
    <button onClick={() => (node.data.viewRef ? openRef(node.data.viewRef)
      : openTask(node.data.subject, { at: node.data.at, ref: node.data.id }))} title={node.title}>
      <AttachmentPreview item={{ ...node.data, ref: node.data.id }} title={node.title} />
    </button>
    {fresh && <i className="hi-work__unopened" role="img" aria-label={L.unopened} />}
  </article>;
  // A group carries its own note as hover text — one line saying what the grouping was
  // based on, so the person reading the chart can see why these three are one thing.
  // **A group opens its own branch**, on this surface: pressed, it becomes the centre, and the
  // group at the centre pressed again steps back out one level.
  // **A group says how much it holds that the chart put away** (`budgeted`), because a group
  // drawing one card and a group that has one card would otherwise look the same.
  if (node.kind === "group") return <article className="hi-work__group" data-node-id={node.id}
    data-kind="group" data-root={root ? "" : undefined} data-more={more ? "" : undefined}
    style={{ "--group-tone": branchPaint(tones.get(node.id), "label") }}>
    <button onClick={() => centreOn(root ? up : node.id)}
      title={[root ? L.back : node.title, node.data.note].filter(Boolean).join(" · ")}>
      {/* A drawn icon whose file has gone since the last arrangement is the default again. */}
      <img className="hi-work__group-icon" src={groupIcon(node.data.icon)} alt="" aria-hidden="true"
        onError={(event) => { if (!event.currentTarget.src.endsWith(DEFAULT_GROUP_ICON)) event.currentTarget.src = DEFAULT_GROUP_ICON; }} />
      <span className="hi-work__group-text">
        <span className="hi-work__group-title">{node.title}</span>
        {more > 0 && <small className="hi-work__group-more">{L.more(more)}</small>}
      </span>
    </button>
  </article>;
  const body = <>
    <span className="hi-work__node-title" title={node.title}>{node.title}</span>
    <div className="hi-work__node-foot"><span className="hi-work__node-state">
      {node.kind === "activity" && <i className="hi-work__live" data-live={node.data.session.state === "running"} />}
      {L.status[state] || ""}</span>
      {["task", "activity"].includes(node.kind) && <time>{age(time, now)}</time>}</div>
  </>;
  return <article className="hi-work__node" data-node-id={node.id} data-kind={node.kind}
    style={{ "--node-tone": TONE[state] || TONE.todo, opacity: emphasis(node, now) }}>
    {open ? <button className="hi-work__open" onClick={open}>{body}</button> : body}
    {fresh && <i className="hi-work__unopened" role="img" aria-label={L.unopened} />}
  </article>;
}

/** The narrow flow's rail is its parent's colour and each tick its own, as a wire would be. */
function Branch({ nodes, parent, ...props }) {
  return <ul className="hi-work__branch" style={{ "--rail": branchPaint(props.tones.get(parent)) }}>
    {nodes.map((node, index) => <li key={node.id} style={{ "--tick": branchPaint(props.tones.get(node.id)),
      "--enter-delay": `${Math.min(index, STAGGER_CAP) * STAGGER_MS}ms` }}>
      <Node node={node} {...props} />
      {props.children.get(node.id)?.length > 0 && <Branch nodes={props.children.get(node.id)} parent={node.id} {...props} />}
    </li>)}</ul>;
}

const CSS = `
/* **The ground is a surface of its own, not the page colour with a hint of tint mixed in.**
   A card is --bg at 84–90% opacity, so a card's lightness is pinned to --bg, and the whole
   job of what lies behind it is therefore to *not be --bg* anywhere a card can land. The wash
   this replaces was built the other way round: two corner radials and a 115deg sweep, every one
   of them mixing a few percent of tint **into** --bg, and the sweep's own midpoint left at a
   literal var(--bg). The radials are anchored top-left and bottom-right and fall to zero at
   72%, the sweep runs the same diagonal — all three layers on one axis — so over the middle of
   the drawing nothing reaches and the sweep is sitting on its midpoint. There the ground was
   --bg, the card was --bg, and a card's contrast against what it sat on was exactly 0%.
   Measured over the ten cards of a real arrangement: |Weber| ran 7% to 42%, one of ten over 40%.

   So the two jobs are split, and that split is the whole of this. --work-ground is a colour
   chosen to be a step away from --bg — away from the *card*, which means lighter on ink and
   darker on paper — and it alone carries legibility. The sweep laid over it carries character
   and nothing else: both its ends are the same oklch L as the ground, so hue swings while
   lightness does not, and what a card gets is a constant instead of a function of where it sits.
   Ten of ten clear 40% now, between -52% and -54% across the whole sweep.

   Ground is the background-color and sweep is the background-image deliberately: if the
   gradient ever fails to parse, what survives is the flat ground, which is the half that
   legibility is in. */
.hi-work { --bg:var(--bg-0); --work-warm:#ff9393; --work-cool:#86c7ed; --work-ground-l:0.933; --work-ground:oklch(var(--work-ground-l) 0.002 82); --work-ground-warm:oklch(var(--work-ground-l) 0.030 19); --work-ground-cool:oklch(var(--work-ground-l) 0.030 232); --work-line:color-mix(in srgb, var(--fg-mute) 25%, var(--bg)); --work-glint:#ffffffd9; --work-shade:#343c502e; --work-shadow:inset 0 1px 0 var(--work-glint), 0 1px 1px #2028380f, 0 10px 20px -6px #20283826, 0 28px 44px -12px var(--work-shade); --work-shadow-raised:inset 0 1px 0 var(--work-glint), 0 1px 1px #20283812, 0 16px 28px -8px #2028382e, 0 40px 64px -16px var(--work-shade); --work-pane:color-mix(in srgb, var(--bg) 72%, transparent); --work-pane-top:color-mix(in srgb, var(--bg) 84%, transparent); --work-frost:blur(18px) saturate(150%); --work-branch-l:0.62; --work-label-l:0.48; --work-branch-c:0.1; height:100%; min-height:0; position:relative; display:flex; flex-direction:column; color:var(--fg); background-color:var(--work-ground); background-image:linear-gradient(115deg, var(--work-ground-warm), var(--work-ground-cool)); padding-top:var(--hi-safe-top, 0px); font-family:var(--font-display, sans-serif); letter-spacing:0; }
.hi-work *, .hi-work *::before, .hi-work *::after { box-sizing:border-box; }
/* The reset stops at the task panel: it is the board's, drawn over this surface, and its buttons
   are styled by the board's own rules, which one class each would lose to these. Under :where so
   the reset weighs what it always did against this surface's own button rules. */
.hi-work button:where(:not(.hi-tasks *)) { font:inherit; color:inherit; background:none; border:0; padding:0; cursor:pointer; text-align:left; }
.hi-work button:where(:not(.hi-tasks *)):focus-visible { outline:2px solid var(--accent); outline-offset:2px; }
.hi-work__error { padding:8px 24px; color:var(--danger); font-size:13px; display:flex; align-items:center; gap:12px; }
.hi-work__error button { text-decoration:underline; min-height:36px; }
.hi-work__frame { flex:1; min-height:0; position:relative; display:flex; flex-direction:column; }
.hi-work__viewport { flex:1; min-height:0; overflow:auto; overscroll-behavior:contain; position:relative; touch-action:pan-x pan-y; padding-bottom:100px; }
.hi-work__viewport[data-chart] { touch-action:none; cursor:grab; }
.hi-work__viewport[data-chart]:active { cursor:grabbing; }
.hi-work__canvas { position:relative; user-select:none; -webkit-user-select:none; }
.hi-work__canvas img { -webkit-user-drag:none; }
.hi-work__stage { position:absolute; transform-origin:0 0; }
/* Visible overflow because a card on its way out is drawn where it stood, which after a refit
   can be outside the new drawing's box, and its wire goes with it. */
.hi-work__wires { position:absolute; left:0; top:0; pointer-events:none; overflow:visible; }
/* Dark mode softens the highlight, deepens the shadow, and leans the panes more opaque. That
   last one used to be read as "transparency buys nothing against a dark wash"; with a ground
   that is deliberately a different lightness from the card it is sharper than that, and points
   the same way. A pane's opacity decides how much ground comes through, and every bit that does
   drags the card *towards* the ground — so on this construction opacity **is** contrast, and 90%
   is a floor to raise rather than a cost to apologise for. The ground's own L is per theme for
   the same reason the panes are: a step away from the card is upward on ink, downward on paper. */
@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) .hi-work { --work-branch-l:0.7; --work-label-l:0.8; --work-branch-c:0.09; --work-glint:#ffffff12; --work-shade:#00000066; --work-pane:color-mix(in srgb, var(--bg) 84%, transparent); --work-pane-top:color-mix(in srgb, var(--bg) 90%, transparent); --work-ground-l:0.330; --work-ground:oklch(var(--work-ground-l) 0.015 87); --work-ground-warm:oklch(var(--work-ground-l) 0.036 28); --work-ground-cool:oklch(var(--work-ground-l) 0.018 223); } }
:root[data-theme="dark"] .hi-work { --work-branch-l:0.7; --work-label-l:0.8; --work-branch-c:0.09; --work-glint:#ffffff12; --work-shade:#00000066; --work-pane:color-mix(in srgb, var(--bg) 84%, transparent); --work-pane-top:color-mix(in srgb, var(--bg) 90%, transparent); --work-ground-l:0.330; --work-ground:oklch(var(--work-ground-l) 0.015 87); --work-ground-warm:oklch(var(--work-ground-l) 0.036 28); --work-ground-cool:oklch(var(--work-ground-l) 0.018 223); }
/* **A wire is structure, so it is drawn like structure.** At 1.8px and 0.8 opacity the
   branch hues came out as pastel threads that a card's border out-weighed, and the one thing
   a wire says — which branch a card belongs to — was the faintest mark on the chart. */
.hi-work__wires path { fill:none; stroke-width:2.6; stroke-linecap:round; opacity:0.95; animation:hi-work-wire-enter 380ms ease var(--enter-delay, 0ms) backwards; }
/* A glide is a transform on the row, from its top-left corner — see useGlide. */
.hi-work__position { position:absolute; transform-origin:0 0; }
/* The core is the same glass one step more solid and one step warmer: it holds the most text
   of anything on the chart, so it is the one pane where the wash behind is a cost. */
.hi-work__core { height:100%; display:flex; flex-direction:column; padding:16px 22px; background:linear-gradient(145deg, color-mix(in srgb, var(--work-warm) 7%, var(--work-pane-top)), var(--work-pane-top) 45%, color-mix(in srgb, var(--work-cool) 6%, var(--work-pane-top))); backdrop-filter:var(--work-frost); -webkit-backdrop-filter:var(--work-frost); border:1px solid var(--work-line); border-radius:14px; }
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
.hi-work__core-more { align-self:flex-start; margin-top:6px; padding:2px 0; font-size:13px; font-weight:500; color:var(--accent); }
.hi-work__core-more:hover { text-decoration:underline; }
/* **A card is a pane of glass over the wash, not a white box on it.** Near-opaque white on a
   near-white page left the separation entirely to a hairline and a shadow nobody could see,
   and the wash — the one thing that says this is a canvas — stopped at the card's edge. A
   blurred, tinted backdrop separates a card by *material*: the colour under it carries
   through, softened, so the card sits above the canvas instead of hiding a patch of it. The
   top-lit gradient is what keeps it a pane and not a smear. */
.hi-work__node { height:100%; background:linear-gradient(155deg, var(--work-pane-top), var(--work-pane)); backdrop-filter:var(--work-frost); -webkit-backdrop-filter:var(--work-frost); border:1px solid var(--work-line); border-radius:12px; display:flex; flex-direction:column; }
.hi-work__node, .hi-work__core, .hi-work__tile { box-shadow:var(--work-shadow); }
/* Entry and exit animate the contents; a glide moves the row by a transform useGlide writes and
   takes off again. None of them touches left/top or the zoom transform. Backwards fill releases
   opacity afterwards so the model's emphasis still owns the settled card. */
.hi-work__node, .hi-work__core, .hi-work__tile, .hi-work__group { animation:hi-work-enter 380ms cubic-bezier(.2,.7,.2,1) var(--enter-delay, 0ms) backwards; }
.hi-work__core { --enter-delay:0ms; box-shadow:var(--work-shadow-raised); }
@keyframes hi-work-enter { from { opacity:0; transform:translateY(10px) scale(.985); } }
/* **A card on its way out fades its own pane, never the row around it.** Opacity below 1 on
   an ancestor makes that ancestor the backdrop root, and the frosted pane inside would blur
   nothing but the row: the card would turn to clear glass for the whole of its exit. */
.hi-work__position[data-leaving] > * { animation:hi-work-exit ${EXIT_MS}ms ease forwards; }
.hi-work__wires path[data-leaving] { animation:hi-work-exit ${EXIT_MS}ms ease forwards; }
@keyframes hi-work-exit { to { opacity:0; } }
@keyframes hi-work-wire-enter { from { opacity:0; } }
/* A heading, not a card: no border and no background, because it is a name over the cards
   below it rather than a thing beside them. */
.hi-work__group { height:100%; font-size:17px; line-height:1.4; font-weight:600; color:var(--group-tone); letter-spacing:0; }
.hi-work__group > button { width:100%; height:100%; display:flex; align-items:center; justify-content:center; gap:8px; padding:0 4px; border-radius:10px; overflow:hidden; overflow-wrap:anywhere; text-align:center; transition:background-color 160ms ease; }
.hi-work__group > button:hover { background:color-mix(in srgb, var(--group-tone) 10%, transparent); }
/* **A heading's content is centred, and its icon still faces its parent.** The box is one
   width for every group, so a three-character label leaves some 50px of slack, and that slack
   is split evenly instead of being pushed outward. The side a group sits on still decides
   which end the icon takes — leading on the chart's right, trailing on its left — so the icon
   is always in the half of the heading nearest the wire that reaches it, though it no longer
   meets it: the wire lands on the box edge, and the icon is now 6-30px inside that edge where
   it was 5px for every group. So siblings no longer line their icons up — across labels of
   three to five characters the column spreads about 24px, each icon moved by half of its own
   label's slack. The centre is centred by this rule now, not as the exception it used to be. */
.hi-work__position[data-dir="-1"] .hi-work__group > button { flex-direction:row-reverse; }
/* **An icon is flat, so nothing around it is not.** The picture carries a background of its
   own, which against the canvas wash cut a hard square out of it; a hairline ring in the
   group's own colour closes that edge and joins the icon to the label beside it. No shadow
   and no gradient here on purpose — every other pane on this chart is glass, and the one
   flat thing on it should stay the one flat thing. */
.hi-work__group-icon { width:40px; height:40px; flex:0 0 40px; border-radius:11px; object-fit:cover; box-shadow:0 0 0 1px color-mix(in srgb, var(--group-tone) 30%, transparent); }
/* The group at the centre is the same heading a size up, standing where the core stood. */
.hi-work__group[data-root] { font-size:22px; }
.hi-work__group[data-root] > button { gap:12px; padding:0 8px; }
.hi-work__group[data-root] .hi-work__group-icon { width:56px; height:56px; flex-basis:56px; border-radius:15px; }
.hi-work__trail { position:absolute; top:12px; left:16px; z-index:3; max-width:calc(100% - 32px); display:flex; flex-wrap:wrap; align-items:center; gap:6px; padding:6px 14px; font-size:13px; line-height:1.4; color:var(--fg-mute); background:var(--work-pane-top); backdrop-filter:var(--work-frost); -webkit-backdrop-filter:var(--work-frost); border:1px solid var(--work-line); border-radius:999px; box-shadow:var(--work-shadow); }
.hi-work__trail-step { display:inline-flex; align-items:center; gap:6px; min-width:0; }
.hi-work__trail button { color:var(--fg-mute); }
.hi-work__trail button:hover { color:var(--fg); text-decoration:underline; text-underline-offset:3px; }
.hi-work__trail strong { color:var(--fg); font-weight:600; }
.hi-work__group-text { min-width:0; display:flex; flex-direction:column; align-items:center; }
.hi-work__group-title { min-width:0; display:-webkit-box; -webkit-line-clamp:2; -webkit-box-orient:vertical; overflow:hidden; text-wrap:balance; }
/* The count takes the second line a long title would have had: 56px holds one line of each. */
.hi-work__group[data-more] .hi-work__group-title { -webkit-line-clamp:1; }
.hi-work__group-more { font-size:13px; line-height:1.35; font-weight:500; opacity:.8; white-space:nowrap; }
.hi-work__open, .hi-work__node { padding:10px 14px; }
/* A tile is the one pane whose contents are opaque, so it is frosted only where the picture
   has not loaded — and it keeps the cards' corner so a rank of both still reads as one rank. */
.hi-work__tile { height:100%; border:1px solid var(--work-line); border-radius:12px; overflow:hidden; background:color-mix(in srgb, var(--fg-mute) 10%, var(--work-pane)); backdrop-filter:var(--work-frost); -webkit-backdrop-filter:var(--work-frost); }
.hi-work__tile button { display:block; width:100%; height:100%; padding:0; }
.hi-work__tile img { display:block; width:100%; height:100%; object-fit:cover; object-position:top center; }
/* Something this task made went up while they were on another page and has not been opened:
   the one red dot on the chart, and it goes the moment the screen is on that page. */
.hi-work__node, .hi-work__tile { position:relative; }
.hi-work__unopened { position:absolute; top:8px; right:8px; width:8px; height:8px; border-radius:50%; background:var(--danger); box-shadow:0 0 0 2px var(--work-pane); pointer-events:none; }
/* A view's shot is taken at the tile's own 16:9 and fills it; a picture or a clip is whatever
   shape the work made it, so it is fitted whole rather than cropped to the box. */
.hi-work__tile[data-shows="picture"] img, .hi-work__tile[data-shows="clip"] img { object-fit:contain; object-position:center; background:color-mix(in srgb, var(--fg) 6%, transparent); }
.hi-work__open { display:flex; flex-direction:column; flex:1; min-width:0; height:100%; padding:0; }
.hi-work__node-title { display:-webkit-box; -webkit-line-clamp:3; -webkit-box-orient:vertical; overflow:hidden; font-size:17px; line-height:1.4; overflow-wrap:anywhere; font-weight:500; }
.hi-work__node-foot { display:flex; flex-wrap:wrap; justify-content:space-between; gap:6px; margin-top:auto; padding-top:8px; font-size:12px; line-height:1.4; color:var(--fg-dim, var(--fg-mute)); }
.hi-work__node, .hi-work__tile { transition:border-color 180ms ease, box-shadow 180ms ease, transform 180ms ease; }
.hi-work__node:focus-within, .hi-work__tile:focus-within { border-color:var(--accent); box-shadow:var(--work-shadow-raised); }
.hi-work__tile:focus-within { outline:2px solid var(--accent); outline-offset:3px; }
@media (hover:hover) and (pointer:fine) {
  .hi-work__node:has(button:hover), .hi-work__tile:has(button:hover) { border-color:color-mix(in srgb, var(--fg-mute) 48%, var(--bg)); box-shadow:var(--work-shadow-raised); transform:translateY(-2px); }
}
.hi-work__node:has(button:active), .hi-work__tile:has(button:active) { transform:translateY(0); box-shadow:var(--work-shadow); }
@media (prefers-reduced-motion:reduce) {
  .hi-work__node, .hi-work__core, .hi-work__tile, .hi-work__group, .hi-work__wires path { animation:none; transition:none; }
  .hi-work__node:has(button:hover), .hi-work__tile:has(button:hover) { transform:none; }
  .hi-work__group > button { transition:none; }
}
.hi-work__node-foot time { white-space:nowrap; }
.hi-work__node-state { display:flex; align-items:center; gap:6px; color:var(--node-tone); }
.hi-work__flow { max-width:720px; margin:0 auto; padding:20px 16px 80px; }
.hi-work__viewport[data-focused] .hi-work__flow { padding-top:64px; }
.hi-work__flow > .hi-work__group { height:auto; min-height:64px; }
.hi-work__flow .hi-work__core { height:auto; min-height:320px; }
/* The narrow flow grades its air by rank for the same reason the chart does: nesting alone
   put a task's own results as far from it as the next task's were. Inheriting --rank-gap
   carries the tightest value on down, so a fourth rank is no looser than the third. */
.hi-work__branch { --rank-gap:30px; list-style:none; padding:0 0 0 18px; margin:0 0 0 8px; border-left:2.6px solid var(--rail, oklch(var(--work-branch-l) 0 0)); }
.hi-work__branch .hi-work__branch { --rank-gap:18px; margin-left:0; padding-left:12px; }
.hi-work__branch .hi-work__branch .hi-work__branch { --rank-gap:10px; }
.hi-work__branch li { position:relative; padding-top:var(--rank-gap); min-width:0; }
.hi-work__branch li::before { content:''; position:absolute; width:18px; left:-18px; top:calc(var(--rank-gap) + 32px); border-top:2.6px solid var(--tick, oklch(var(--work-branch-l) 0 0)); }
.hi-work__branch .hi-work__node { min-height:116px; }
.hi-work__branch .hi-work__group { height:auto; min-height:28px; }
.hi-work__branch .hi-work__tile { height:auto; aspect-ratio:16 / 9; max-width:240px; }
.hi-work__branch .hi-work__branch li::before { left:-12px; width:12px; }
.hi-work__loading { position:absolute; left:24px; top:8px; color:var(--fg-mute); font-size:13px; z-index:2; }
@media (max-width:759px) { .hi-work__node-title { min-height:36px; } }
`;
