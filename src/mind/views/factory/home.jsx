// purpose: 现在 — the agent's open work drawn as one chart: what it owes, grouped by what
// the work is about, with who is on each thing and what it has had its head in, hanging off
// a single hub. The surface for when nothing more specific is up, which is most of the time.
//
// **Why it exists.** A show puts up an artifact and an artifact exists at the *end*, so the
// interval in which the person most wants something on the screen — the agent is working,
// they are waiting — is exactly the interval in which nothing is. Two arrivals land on that
// screen and neither is answered: *I asked for this fifteen minutes ago* (which row, is
// anybody on it, what did that session last do, how long ago) and *where is everything*
// (what is owed, what is waiting on **them**, what closed, what it has been thinking about).
//
// Everything both need is on the wire already, and drawn already, in three places none of
// which is on the screen. `tasks` is the ledger, `workers` is the session tree, and
// `GET /api/activity` is what the face reads to choose between Working and Idle. The join a
// person performs by hand — open the band, read a card that says nobody is on it, go to
// Sessions, look for a session whose title mentions it — is one field, `subject`. This page
// is that join, drawn.
//
// **Why a chart and not a list.** The first build of this page was a column of cards and it
// was a good board and a bad answer: twelve cards in a column is `tasks.jsx` with fewer
// columns, and the one thing it could not show is the thing this page is *for* — that these
// rows are not peers. A chart draws that as **shape**: where a branch sits and how far it
// reaches say what a sentence would otherwise have to.
//
//     ⟨gz⟩ deploy KUT to production ─┐                ┌─ 制作 20 周健康训练打卡 view ⟨YOU⟩
//              KUT COS storage backend ├─ KUT 4 ─┐    ├─ hi-agent 3 ─ 研究 Agent 相关性分类器
//            转码回归 — KUT 上传后掉帧 2● ─┘         ●        └─ 2026 中秋国庆返川行程 view
//                          生词本自动收录 ─┘  hub    └─ feishu 1 ─ Feishu weekly digest ●
//
// **It is organised by subject, not by state.** An earlier draft made the trunks `on you` /
// `nobody on it` / `running` / `queued`, which reads well and is the wrong axis: it answers
// *what needs me* and destroys *what am I working on*. State did not disappear, it moved
// down a rank — the leaf carries the tone and the one fact, and a topic takes the colour of
// the hottest leaf under it, so `KUT` glows red because three rows under it have nobody on
// them without state being what the chart is built out of.
//
// ── what a row is about, which is not a field ────────────────────────────────
//
// There is no `project` on a task. What there is is `systems:` — free prose the mind writes
// while opening a row, arriving here in `extra` because the schema does not know the key.
//
// Read naively it is useless. On the live store this was measured against, `hi-agent` sat on
// **nine of the nine** open rows that carry the key at all: the agent works on itself, so
// the tag is on everything and therefore distinguishes nothing. Grouping by first-tag, or by
// rarest-tag, both produce a wrong chart — one node holding the world, or `kt8-092` filed
// under `gz` because a deployment region happened to be mentioned once.
//
// **So a tag on more than half the tagged rows is ambient, not a topic.** A branch holding
// more than half the chart is not a branch, it is the trunk. Drop those — unless dropping
// them leaves a row with nothing, in which case the ambient tag genuinely is what that row
// is about, and three rows on the live store are exactly that: work on the agent itself.
//
// **Depth is read out of the tags, not fixed at two.** A row can be a lone errand under
// nothing, or a task under a part of a project under the project, and a chart that always
// draws exactly two ranks is asserting a shape the work does not have. The containment is
// already in the data: **B is inside A when every open row carrying B also carries A, and A
// holds strictly more rows.** `Tencent COS` appears only on rows that are also `KUT`, so it
// is part of KUT; `wecom` and `songguo` share no rows, so they are peers.
//
// **A topic has to be worth a rank: three rows, or a project the agent keeps a facet on.**
// The count is the ordinary case — a heading over one line is not a grouping, and a
// sub-topic covering one row puts an empty level between a project and a single task, saying
// only what the row's own title already said. The exception is the one a count cannot see: a
// long-running thread with a clear boundary can be down to its last row and still be the
// thing that row is under, and the record of that is already on disk in `facets/projects/`.
// What does not earn a node dissolves upward and its name rides on the rows as a mark, so
// nothing is lost — `deploy KUT to production ⟨gz⟩` says what a `gz` branch would have, in
// the place the reader is already looking.
//
// **Nothing is bridged, only folded.** Case and `-`/`_`/space are notation and code may
// settle them; `wechat` and `wecom` are two systems and this never joins them
// (`docs/arch/data.md`). And a row tagged with nothing hangs off the hub — not filed under
// "misc", which would claim a kinship the store does not record.
//
// ── what is drawn ───────────────────────────────────────────────────────────
//
// **Three edges, and every one is stored.** `task ← session` is the worker's `subject`,
// which `CreateWorker` requires and refuses when nothing is filed under it. `session ←
// session` is `owner`, an id. `task → artifact` is the record's own inline-code tokens
// resolved against its folder. Nothing here is inferred and nothing is drawn that a field
// does not say.
//
// **The centre carries nothing, on purpose.** It has been three things and each was worse
// than the last for the same reason: whatever went into the largest type on the surface was
// already drawn on the branches. `working · 4 sessions · 14 open` was arithmetic over the
// trunk counts and the marks on the rows. The live session's own words — `pytest -k live
// tests/transcode` — were higher-information and *less* meaning: a shell fragment at 26px,
// describing one row out of fourteen, with no way for a reader to tell why that one. A hub
// is what a mind map's centre actually is when the map is the content. Two things still
// speak there and both are absences rather than summaries: the pulse, because whether
// anything at all is moving is on no branch; and the one failure no row can show — with no
// Reaction registered the agent cannot hear you, however busy the chart looks.
//
// **One fact per leaf**, in the order `tasks.jsx` arrived at for its cards: a `waiting` line
// verbatim if a human is the blocker; else what the session on it is doing and how long
// since; else **nobody on it** and how long it has stood there; else last-confirmed-alive for
// a duty; else age. The third is the reason to build this — nothing on any current surface
// says it, and a restart's casualties are not in the roster at all, so a row whose worker
// died reads as work in progress on the board and as nothing anywhere else.
//
// **Who is on a row is a mark on the row**, not a rank of its own. It was a fourth rank of
// pills once: a column of chips floating in the right of the window, wired back to their
// rows because nothing else said whose they were, saying `general running` beside a sentence
// that already read `pytest -k live · 1m`. What the pills alone knew was *how many* and
// *what kind* — a numeral and a word, both of which fit on the row.
//
// **Nothing on it is a control.** A leaf cannot open the board on its own row, because
// `POST /api/views/open` carries a ref and no argument; the fix is one optional field on that
// request, general to every pair of factory views, and it is not here.
//
// ── the frame ───────────────────────────────────────────────────────────────
//
// **Two arms, because one was a chart pinned to the left edge of a wide screen.** A mind
// map's centre is in the middle and its branches go both ways; that is not decoration, it is
// what makes the width work. Below 1200px it keeps one arm, and below 960px it stops being a
// chart at all — a dendrogram needs width per rank and a phone has one rank's worth, so the
// same model renders as an indented flow there. Desktop is the case this page was designed
// for and the narrow one is correct rather than considered.
//
// **And it is laid out to the frame, in two levers.** Air first: the layout runs once at its
// natural spacing, and if that leaves the frame mostly empty the leftover height is divided
// between the rows and it runs again — air between rows, never taller rows, so nothing
// written on the map changes size. Then scale: air alone cannot help a four-row map on a
// tall screen, so the map reports how much of the frame it occupies and is scaled up to fill
// the rest, capped. A scale rather than a bigger type scale, because every proportion here
// was decided against the others and a sparse map should be the same drawing seen larger.
//
// **Twelve branches, not a hundred and forty-nine.** The store this was designed against
// carries 149 rows — 3 todo, 5 doing, 4 serving, 108 done, 25 cancelled — so 8% of the ledger
// is open. That number is the layout. Closed work is one leaf carrying two counts;
// `tasks.jsx` already learned the expensive version of this when five equal columns spent 40%
// of the width on the 93% of rows that had closed.
//
// **What this cannot do yet, in the file rather than after the fact:**
//   - The project trunk costs one request per project, because `GET /api/facets` returns
//     sorted names and no timestamps. It reads on attention rather than on a clock for
//     exactly that reason. A `modified` on the list endpoint removes it.
//   - Its tile in the band is a picture of this file's loading state. `settle()` in the
//     render page waits two frames, fonts and images, and not for a fetch, so the shot is
//     taken before the first read comes back. That is every data-driven surface here
//     (`_shots/ref/factory/tasks.png` is an empty board reading "0 active"), and it lands
//     hardest on this one, which is the surface a person is most likely to reach for cold.
//   - It is not walkable on a television. Spatial navigation runs in the host's cover plane;
//     a view's own arrows are its own and this view binds none. The rank geometry is chosen
//     for the day that lands — ranks walk under a D-pad — but nothing has been read from
//     across a room.
//   - **Nothing in the host decides when it goes up**, and nothing should: `stage.md` refuses
//     a host gate on what is on the screen. It is a view like any other — reachable from the
//     bookmarks row, shown by `hi_show`. When Reaction reaches for it is guidance, and it
//     lives in `identity/reaction.md` beside the rule it completes: *an old answer left
//     standing says you are still on it*, which until now had only two endings and both were
//     bad — leave the stale view up, or dismiss to an empty room. It is where the screen
//     **rests**: up when the subject is the state of the work rather than any one piece of
//     it, and replaced the moment a particular thing exists to put there.
//
// Colour comes from the host theme tokens (see tasks.jsx for the vocabulary).
import { useCallback, useEffect, useRef, useState } from "react";
import { useLive, TEMPO } from "@hi/core";

// ── words ─────────────────────────────────────────────────────────────────────
// English is the default and the fallback. Task is 任务 and Sessions is 会话 — plain words,
// which translate. What a session is *doing* is written in plain words too: `general` is the
// default worker type and `view-builder` is this system's own name for a specialism, and
// neither says anything to somebody watching their own errand.
//
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped here.
// Until that exists, an unsupported language lands on English.
const T = {
  en: {
    reading: "Reading the ledger...",
    nothingOpen: "Nothing is open. The ledger is clear.",
    deaf: "not listening",
    trunk: { minds: "also on its mind", closed: "closed" },
    you: "you",
    nobody: "nobody on it",
    stood: (age) => `standing ${age}`,
    alive: (age) => `confirmed alive ${age} ago`,
    neverChecked: "never confirmed alive",
    since: (age) => `${age} ago`,
    closedN: (day, week) => `${day} today · ${week} this week`,
    noMinds: "nothing in memory yet",
    doing: {
      general: "working on it",
      "view-builder": "building a view",
      "view-reviewer": "reviewing it",
      "decision-maker": "making a call",
      "drive-organizer": "filing it",
      "person-reader": "reading up on someone",
      "task-manager": "keeping the ledger",
    },
    parked: { idle: "idle", waiting: "waiting" },
  },
  zh: {
    reading: "正在读账...",
    nothingOpen: "没有开着的事。",
    deaf: "听不见",
    trunk: { minds: "还想着", closed: "已结束" },
    you: "等你",
    nobody: "没人在上面",
    stood: (age) => `已经放了 ${age}`,
    alive: (age) => `${age}前确认还活着`,
    neverChecked: "还没确认过活着",
    since: (age) => `${age}前`,
    closedN: (day, week) => `今天 ${day} 件 · 本周 ${week} 件`,
    noMinds: "记忆里还没有东西",
    doing: {
      general: "在做",
      "view-builder": "在做视图",
      "view-reviewer": "在审",
      "decision-maker": "在拿主意",
      "drive-organizer": "在归档",
      "person-reader": "在读这个人",
      "task-manager": "在记账",
    },
    parked: { idle: "闲着", waiting: "等着" },
  },
};

// App setting first — the host puts it on `<html lang>` — then the system locale when that
// setting says to follow the person, then English.
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
const ZH = L === T.zh;

const OPEN = new Set(["todo", "doing", "serving"]);
const MINDS_SHOWN = 8;
// A ceiling on the per-project reads, so a store that grows a hundred projects does not turn
// one attention into a hundred requests.
const MINDS_READ = 60;
// A topic earns a rank at this many rows, or by being a project the agent keeps a facet on.
const EARNS = 3;

// Two of these are shouted and five are not. Red is worth exactly as much as it is spent
// sparingly: `wait` and `warn` are the states that want a person, and everything else is the
// page being informative rather than alarming. An earlier draft painted the fact itself in
// both, and on a live store that put five of twelve rows in full red — a page with no
// emphasis left to spend.
const TONE = {
  wait: "var(--danger)",
  warn: "var(--danger)",
  live: "var(--accent)",
  serving: "var(--accent-2)",
  todo: "var(--fg-mute)",
  minds: "var(--fg-mute)",
  closed: "var(--fg-mute)",
};
// A topic is as hot as the hottest thing under it: that is how a chart organised by subject
// still answers *what needs me* at a glance, which organising by state answered for free.
const HEAT = ["wait", "warn", "live", "serving", "todo", "minds", "closed"];
const hotter = (a, b) => (HEAT.indexOf(a) <= HEAT.indexOf(b) ? a : b);

// ── reading ───────────────────────────────────────────────────────────────────

const api = {
  tasks: () => fetch("/api/tasks").then((r) => r.json()),
  // The switchboard, joined to the ledger by subject in this view rather than served
  // alongside it — the same join `tasks.jsx` makes, for the same reason: `GET /api/workers`
  // already carries `subject`, so the alternative is a second derivation to keep agreeing
  // with this one forever. What it costs is precision this page does not spend: a restart's
  // casualties are not in the roster at all, so cut-off work reads "nobody on it" rather
  // than naming the restart.
  workers: () => fetch("/api/workers").then((r) => r.json()),
  facets: () => fetch("/api/facets").then((r) => r.json()),
  facet: (subject) =>
    fetch(`/api/facets/projects/${encodeURIComponent(subject)}`).then((r) => r.json()),
};

// ── derivations ───────────────────────────────────────────────────────────────

// The newest thing a *mind* wrote. `moved` is the store's, so a row that only changed status
// has said nothing and the line under it is still the current one.
function latestSpoken(task) {
  const timeline = task.timeline || [];
  for (let i = timeline.length - 1; i >= 0; i -= 1) {
    if (timeline[i].kind !== "moved") return timeline[i];
  }
  return null;
}

// Whether a human is wanted right now. Nothing clears a `waiting` line — it is a dated
// sentence in a record that only appends — so what makes it current is that no mind has
// written anything under it.
function waitsOnPerson(task) {
  if (!OPEN.has(task.status)) return false;
  return latestSpoken(task)?.kind === "waiting";
}

function stamp(value) {
  const at = value ? new Date(value) : null;
  return at && !Number.isNaN(at.getTime()) ? at : null;
}

// Coarse on purpose: this is read across a room, and a figure that ticks every second on a
// screen nobody is touching is motion without information.
function ago(value) {
  const at = stamp(value);
  if (!at) return null;
  const mins = Math.max(0, Math.round((Date.now() - at.getTime()) / 60000));
  if (mins < 1) return ZH ? "刚刚" : "just now";
  if (mins < 60) return ZH ? `${mins} 分钟` : `${mins}m`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return ZH ? `${hours} 小时` : `${hours}h`;
  const days = Math.round(hours / 24);
  return ZH ? `${days} 天` : `${days}d`;
}

// Every live session on a row, running first, then by how recently it moved. `tasks.jsx`
// keeps one per subject because a card has room for one; a row here can say how many, and
// two sessions on one row is a fact worth seeing rather than a tie to break.
function crewBySubject(workers) {
  const map = new Map();
  for (const worker of workers) {
    if (!worker.subject) continue;
    const held = map.get(worker.subject);
    if (held) held.push(worker);
    else map.set(worker.subject, [worker]);
  }
  const rank = (w) => (w.state === "running" ? 2 : w.state === "waiting" ? 1 : 0);
  for (const list of map.values()) {
    list.sort(
      (a, b) =>
        rank(b) - rank(a) ||
        (stamp(b.state_since)?.getTime() || 0) - (stamp(a.state_since)?.getTime() || 0),
    );
  }
  return map;
}

// The one fact, and the tone that carries it. The order is the reader's question — *does
// this need me* — and not the record's shape. Derived here and nowhere else, so the colour
// of a leaf and the sentence on it cannot disagree.
function read(task, crew) {
  const wait = waitsOnPerson(task) ? latestSpoken(task) : null;
  if (wait) return { tone: "wait", text: wait.text };

  const live = (crew || []).find((w) => w.state === "running");
  if (live) {
    // `doing` is only meaningful while the session is running — nothing clears it when a turn
    // ends, and drawing it beside a finished session is what made one read `idle` and
    // `thinking` at once.
    const age = ago(live.doing_at || live.state_since);
    const doing = live.doing || live.tail;
    return { tone: "live", text: doing ? `${doing}${age ? ` · ${age}` : ""}` : "" };
  }

  if (task.status === "serving") {
    const age = ago(task.checkedAt);
    // A duty that has never reported is the one `serving` row that is an alarm.
    return age ? { tone: "serving", text: L.alive(age) } : { tone: "warn", text: L.neverChecked };
  }

  if (task.status === "doing") {
    const age = ago(task.statusSince);
    return { tone: "warn", text: age ? `${L.nobody} · ${L.stood(age)}` : L.nobody };
  }

  const age = ago(task.statusSince || task.createdAt);
  return { tone: "todo", text: age ? L.since(age) : "" };
}

function closedCounts(tasks) {
  const now = Date.now();
  let day = 0;
  let week = 0;
  for (const task of tasks) {
    if (OPEN.has(task.status)) continue;
    const at = stamp(task.completedAt || task.cancelledAt || task.statusSince);
    if (!at) continue;
    const hours = (now - at.getTime()) / 3600000;
    if (hours <= 24) day += 1;
    if (hours <= 24 * 7) week += 1;
  }
  return { day, week };
}

// ── topics ────────────────────────────────────────────────────────────────────

const FOLD = /[\s_]+/g;
const fold = (raw) => String(raw || "").trim().toLowerCase().replace(FOLD, "-");

/** `project:` first — nothing writes it today, and if anything ever does it is a better
 *  answer than prose about systems. Then `systems:`, split on commas. */
function tags(task) {
  const field = (key) => (task.extra || []).find((f) => f.key === key)?.value;
  const raw = field("project") || field("systems") || "";
  return raw
    .split(",")
    .map((piece) => piece.trim())
    .filter(Boolean);
}

/** Fold every open row's tags into a forest of topics and say which node each row hangs off.
 *  Returns plain data; nothing here knows the chart exists. */
function forest(open, projectNames) {
  const rows = new Map(); // folded tag -> { label, ids:Set }
  const carried = new Map(); // task subject -> folded[]
  let tagged = 0;

  for (const { task } of open) {
    const list = tags(task);
    if (list.length === 0) continue;
    tagged += 1;
    const folded = [];
    for (const raw of list) {
      const key = fold(raw);
      if (!key || folded.includes(key)) continue;
      folded.push(key);
      const held = rows.get(key);
      if (held) held.ids.add(task.subject);
      else rows.set(key, { label: raw.trim(), ids: new Set([task.subject]) });
    }
    carried.set(task.subject, folded);
  }
  // A project facet's own spelling wins over whatever the record typed, so the chart and
  // memory call the same thing by the same name.
  for (const name of projectNames) {
    const held = rows.get(fold(name));
    if (held) held.label = name;
  }

  const ambient = (key) => tagged > 1 && rows.get(key).ids.size * 2 > tagged;
  for (const [subject, folded] of carried) {
    const useful = folded.filter((key) => !ambient(key));
    carried.set(subject, useful.length > 0 ? useful : folded);
  }
  // Ambient tags survive only where a row had nothing else; anything now carried by nobody is
  // not a topic at all, and every remaining row set has to be recounted without them.
  const live = new Set([...carried.values()].flat());
  for (const key of [...rows.keys()]) {
    if (!live.has(key)) rows.delete(key);
    else rows.get(key).ids = new Set();
  }
  for (const [subject, folded] of carried) {
    for (const key of folded) rows.get(key).ids.add(subject);
  }

  // Containment: the tightest strict superset is the parent.
  const parent = new Map();
  for (const [key, node] of rows) {
    let best = null;
    for (const [other, cand] of rows) {
      if (other === key || cand.ids.size <= node.ids.size) continue;
      if (![...node.ids].every((id) => cand.ids.has(id))) continue;
      if (!best || cand.ids.size < rows.get(best).ids.size) best = other;
    }
    if (best) parent.set(key, best);
  }
  const depth = (key) => {
    let n = 0;
    let at = key;
    while (parent.has(at)) {
      at = parent.get(at);
      n += 1;
    }
    return n;
  };

  // Each row hangs off the deepest tag it carries; the widest of those wins a tie, because a
  // tie means two tags cover exactly the same rows and the commoner name is the topic.
  const home = new Map();
  for (const [subject, folded] of carried) {
    const pick = folded
      .slice()
      .sort(
        (a, b) =>
          depth(b) - depth(a) ||
          rows.get(b).ids.size - rows.get(a).ids.size ||
          Number(projectNames.has(rows.get(b).label)) - Number(projectNames.has(rows.get(a).label)) ||
          folded.indexOf(a) - folded.indexOf(b),
      )[0];
    home.set(subject, pick);
  }
  return { rows, parent, home };
}

/** The chart's content, before it has any geometry: a forest of topics with the rows under
 *  each, then the rows that belong to nothing, then memory and the archive. */
function model(tasks, workers, minds) {
  const crew = crewBySubject(workers);
  const open = [];
  for (const task of tasks) {
    if (!OPEN.has(task.status)) continue;
    const on = crew.get(task.subject) || [];
    const one = read(task, on);
    open.push({
      task,
      leaf: {
        id: task.subject,
        title: task.title || task.subject,
        fact: one.text,
        tone: one.tone,
        sessions: on,
      },
    });
  }

  const projectNames = new Set(minds.map((m) => m.subject));
  const { rows, parent, home } = forest(open, projectNames);

  let roots = [];
  const nodes = new Map();
  for (const [key, row] of rows) {
    nodes.set(key, { key, label: row.label, children: [], leaves: [] });
  }
  for (const [key, node] of nodes) {
    const up = parent.get(key);
    if (up && nodes.has(up)) nodes.get(up).children.push(node);
    else roots.push(node);
  }
  const loose = [];
  for (const row of open) {
    const key = home.get(row.task.subject);
    if (key && nodes.has(key)) nodes.get(key).leaves.push(row.leaf);
    else loose.push(row.leaf);
  }

  const earns = (node) =>
    node.leaves.length + node.children.reduce((n, c) => n + c.count, 0) >= EARNS ||
    projectNames.has(node.label);

  const settle = (node) => {
    const kept = [];
    for (const child of node.children) {
      settle(child);
      // Nothing under it at all: three tags on one row are mutually contained and none of
      // them is anybody's parent, so two of them end up naming no work. A topic with no rows
      // under it is not a topic, it is a word the record happened to use.
      if (child.children.length === 0 && child.leaves.length === 0) continue;
      if (earns(child)) {
        kept.push(child);
        continue;
      }
      for (const leaf of child.leaves) {
        leaf.marks = [child.label, ...(leaf.marks || [])];
        node.leaves.push(leaf);
      }
      // A grandchild that earned its own node keeps it, one rank shallower.
      for (const grand of child.children) kept.push(grand);
    }
    node.children = kept;
    node.count = node.leaves.length + node.children.reduce((n, c) => n + c.count, 0);
    node.tone = [...node.leaves.map((l) => l.tone), ...node.children.map((c) => c.tone)].reduce(
      hotter,
      "closed",
    );
    node.leaves.sort((a, b) => HEAT.indexOf(a.tone) - HEAT.indexOf(b.tone));
    node.children.sort((a, b) => HEAT.indexOf(a.tone) - HEAT.indexOf(b.tone) || b.count - a.count);
  };
  for (const node of roots) settle(node);

  const standing = [];
  for (const node of roots) {
    if (node.children.length === 0 && node.leaves.length === 0) continue;
    if (earns(node)) {
      standing.push(node);
      continue;
    }
    for (const leaf of node.leaves) {
      leaf.marks = [node.label, ...(leaf.marks || [])];
      loose.push(leaf);
    }
    for (const grand of node.children) standing.push(grand);
  }
  standing.sort(
    (a, b) =>
      HEAT.indexOf(a.tone) - HEAT.indexOf(b.tone) ||
      b.count - a.count ||
      a.label.localeCompare(b.label),
  );
  roots = standing;

  if (loose.length > 0) {
    loose.sort((a, b) => HEAT.indexOf(a.tone) - HEAT.indexOf(b.tone));
    // No label and no node: these hang straight off the hub.
    roots.push({ key: "__loose", label: null, count: null, tone: "todo", children: [], leaves: loose });
  }

  const idle = minds.filter((m) => !nodes.has(fold(m.subject)));
  if (idle.length > 0) {
    roots.push({
      key: "__minds",
      label: L.trunk.minds,
      count: null,
      tone: "minds",
      children: [],
      leaves: [{ id: "__minds", kind: "chips", tone: "minds", chips: idle.slice(0, MINDS_SHOWN) }],
    });
  }
  const closed = closedCounts(tasks);
  if (closed.week > 0) {
    roots.push({
      key: "__closed",
      label: L.trunk.closed,
      count: null,
      tone: "closed",
      children: [],
      leaves: [
        { id: "__closed", kind: "note", tone: "closed", title: L.closedN(closed.day, closed.week) },
      ],
    });
  }
  return roots;
}

// ── geometry ──────────────────────────────────────────────────────────────────
// Nothing but numbers. Every node position and every edge endpoint on the chart is read off
// what this returns, so a label and the line reaching it cannot drift apart.

const ROW = 44; // a leaf with a fact under it
const ROW_TIGHT = 30; // a note leaf, which is one line
const CHIP_ROW = 27; // one row of project chips
const TRUNK_GAP = 26;
const PAD_TOP = 26;

// Chips wrap, so the chips leaf is the one node whose height is not a constant. Estimated
// from the text rather than measured: a leaf that claimed one row and drew three overlapped
// the trunk under it, and a layout pass that has to wait for a DOM measurement to know where
// anything goes is a pass that draws the chart wrong once on every mount.
const CHIP_CHAR = 7.2;
const CHIP_PAD = 62;
function chipRows(chips, width) {
  let rows = 1;
  let x = 0;
  for (const chip of chips) {
    const w = chip.subject.length * CHIP_CHAR + CHIP_PAD;
    if (x > 0 && x + w > width) {
      rows += 1;
      x = 0;
    }
    x += w + 6;
  }
  return rows;
}

/** Roughly how wide a string draws, in the same spirit as the chip rows: estimated, for the
 *  same reason. CJK is close to square at a given size; latin is a bit over half. */
function estWidth(text, px) {
  let w = 0;
  for (const ch of String(text || "")) w += ch.codePointAt(0) > 0x2e80 ? px : px * 0.56;
  return w;
}

/** The widest a leaf actually needs, so a map of short rows does not claim the frame's full
 *  measure and then sit in the middle of it. */
function needed(nodes) {
  let widest = 0;
  for (const node of nodes) {
    for (const leaf of node.leaves) {
      const marks = (leaf.marks || []).reduce((n, m) => n + estWidth(m, 10.5) + 20, 0);
      const dot = leaf.sessions?.length ? 22 : 0;
      widest = Math.max(
        widest,
        estWidth(leaf.title, 14) + marks + dot,
        estWidth(leaf.fact, 11.5) + (leaf.tone === "wait" ? 34 : 0),
        leaf.kind === "chips" ? 420 : 0,
      );
    }
    widest = Math.max(widest, needed(node.children));
  }
  return widest;
}

/** Depth of the deepest topic node, so the leaf column clears every rank above it. */
function ranks(nodes, at = 1) {
  let deepest = at;
  for (const node of nodes) {
    if (node.children.length > 0) deepest = Math.max(deepest, ranks(node.children, at + 1));
  }
  return deepest;
}

/** Every leaf, so the spread below has something to divide by. */
function leafCount(nodes) {
  return nodes.reduce((n, node) => n + node.leaves.length + leafCount(node.children), 0);
}

/** How tall a trunk will draw, before anything is placed. Only needed to decide which arm it
 *  goes on, which has to happen before the placing. */
function extent(node) {
  const own = node.leaves.reduce(
    (n, leaf) =>
      n + (leaf.kind === "chips" ? chipRows(leaf.chips, 420) * CHIP_ROW + 6 : leaf.kind ? ROW_TIGHT : ROW),
    0,
  );
  return own + node.children.reduce((n, child) => n + extent(child), 0);
}

/** Deal the trunks onto two arms, in order, each going to whichever arm is shorter.
 *
 *  Order is preserved rather than optimised: the trunks arrive sorted by heat and that is the
 *  one thing about their sequence a reader can use, so the hottest still lands at the top of
 *  an arm. Balancing only decides *which* arm. */
function deal(trunks) {
  const left = [];
  const right = [];
  let lh = 0;
  let rh = 0;
  for (const trunk of trunks) {
    const h = extent(trunk);
    if (rh <= lh) {
      right.push(trunk);
      rh += h;
    } else {
      left.push(trunk);
      lh += h;
    }
  }
  return { left, right };
}

function shift(nodes, dy) {
  for (const node of nodes) {
    node.y += dy;
    for (const leaf of node.leaves) {
      leaf.y += dy;
      leaf.titleY += dy;
    }
    shift(node.children, dy);
  }
}

function chart(trunks, width, avail, spread = 0) {
  const twoArmed = width >= 1200;
  const hubX = twoArmed ? Math.round(width * 0.5) : 58;
  const arm = Math.round(Math.min(132, Math.max(86, width * 0.062)));
  const step = Math.round(Math.min(150, Math.max(100, width * 0.078)));
  const { left, right } = twoArmed ? deal(trunks) : { left: [], right: trunks };

  const lay = (nodes, dir) => {
    const depth = ranks(nodes);
    const leafX = hubX + dir * (arm + depth * step);
    const room = (dir > 0 ? width - leafX : leafX) - 34;
    const leafW = Math.round(Math.min(760, Math.max(210, Math.min(room, needed(nodes) + 12))));
    let y = PAD_TOP;
    const place = (ns, rank) =>
      ns.map((node) => {
        const leaves = node.leaves.map((leaf) => {
          const base =
            leaf.kind === "chips"
              ? chipRows(leaf.chips, leafW) * CHIP_ROW + 6
              : leaf.kind
                ? ROW_TIGHT
                : ROW;
          // The row keeps its own height and gets the spread as air around it, so opening the
          // map out fills the frame without stretching anything written on it.
          const h = base + spread;
          // Where the leaf's *first line* sits, which is what a mark on the row hangs off.
          const laid = { ...leaf, y: y + h / 2, titleY: y + spread / 2 + 11, h };
          y += h;
          return laid;
        });
        const children = place(node.children, rank + 1);
        const own = [...leaves, ...children];
        const top = own[0]?.y ?? y;
        const bottom = own[own.length - 1]?.y ?? y;
        if (rank === 1) y += TRUNK_GAP + spread;
        return {
          ...node,
          x: hubX + dir * (arm + (rank - 1) * step),
          y: (top + bottom) / 2,
          leaves,
          children,
        };
      });
    const laid = place(nodes, 1);
    return { dir, leafX, leafW, trunks: laid, height: Math.max(y - TRUNK_GAP - spread, PAD_TOP) };
  };

  const arms = [lay(right, 1)];
  if (left.length > 0) arms.push(lay(left, -1));

  // Both arms hang from the same hub, so the shorter one is centred against the taller rather
  // than starting at the top and trailing off.
  const height = Math.max(...arms.map((a) => a.height));
  for (const a of arms) shift(a.trunks, (height - a.height) / 2);

  // How much of the frame the map actually occupies, so it can be opened out to fill the
  // rest. A four-row map on a tall screen is a cluster adrift in a field otherwise: the
  // spread can only add air between rows, and four rows cannot spend a thousand pixels of it
  // without looking abandoned.
  const usedW =
    Math.max(...arms.map((a) => Math.abs(a.leafX - hubX) + a.leafW)) * (arms.length > 1 ? 2 : 1) +
    (arms.length > 1 ? 0 : hubX);
  const geo = {
    hubX,
    hubY: (height + PAD_TOP) / 2,
    width,
    height: height + PAD_TOP,
    arms,
    scale: Math.max(
      1,
      Math.min(1.5, (width - 24) / Math.max(usedW, 1), avail / Math.max(height + PAD_TOP, 1)),
    ),
  };

  // A second pass, once, to fill the frame. Air is the better of the two levers while there
  // is a sensible amount to add, so it goes first; bounded to one retry, because the second
  // pass changes the height it was computed from and chasing that is a loop.
  if (spread === 0 && avail > geo.height + 24) {
    const n = Math.max(leafCount(trunks), 1);
    const room = Math.min(46, Math.floor((avail - geo.height) / n));
    if (room >= 3) return chart(trunks, width, avail, room);
  }
  return geo;
}

/** A branch: out of `x1,y1` horizontally, into `x2,y2` horizontally. The control points sit
 *  at the midpoint so every curve in the chart has the same tension whatever it spans. */
function branch(x1, y1, x2, y2) {
  const mid = (x1 + x2) / 2;
  return `M ${x1} ${y1} C ${mid} ${y1}, ${mid} ${y2}, ${x2} ${y2}`;
}

// ── the surface ───────────────────────────────────────────────────────────────

export default function Home() {
  const [tasks, setTasks] = useState(null);
  const [workers, setWorkers] = useState([]);
  const [minds, setMinds] = useState([]);
  const frame = useRef(null);
  const [box, setBox] = useState({ w: 0, h: 0 });

  const load = useCallback(async () => {
    const [ledger, roster] = await Promise.all([
      api.tasks().catch(() => null),
      api.workers().catch(() => null),
    ]);
    // A read that does not come back leaves the last good one standing: empty is a claim
    // ("nobody is on anything") and a failed fetch is not entitled to make it.
    if (ledger) setTasks(ledger.tasks || []);
    else setTasks((prev) => (prev === null ? [] : prev));
    if (roster) setWorkers(roster.workers || []);
  }, []);

  // The chart is something you watch happen. The project trunk is not, and it costs one
  // request per project, so it reads when someone looks at the page rather than on a clock.
  const loadMinds = useCallback(async () => {
    const index = await api.facets().catch(() => null);
    if (!index) return;
    const dimension = (index.dimensions || []).find((d) => d.dimension === "projects");
    const subjects = (dimension?.subjects || []).slice(0, MINDS_READ);
    const seen = await Promise.all(
      subjects.map((subject) =>
        api
          .facet(subject)
          .then((f) => ({ subject, at: stamp(f?.modified) }))
          .catch(() => ({ subject, at: null })),
      ),
    );
    seen.sort((a, b) => (b.at?.getTime() || 0) - (a.at?.getTime() || 0));
    setMinds(seen);
  }, []);

  useLive(load, { period: TEMPO.watching });
  useLive(loadMinds, { period: TEMPO.onAttention });

  // The chart is drawn to the frame it is handed, and the frame is the window — both ways.
  // Measured rather than assumed: this view is mounted at four widths that matter (a maximised
  // Mac, a half-width column, the headless renderer, a phone), a media query cannot hand
  // JavaScript a number to lay out with, and the height is what decides how far it opens out.
  useEffect(() => {
    const el = frame.current;
    if (!el || typeof ResizeObserver === "undefined") {
      setBox({ w: el?.clientWidth || 1200, h: el?.clientHeight || 800 });
      return;
    }
    const ro = new ResizeObserver(([entry]) =>
      setBox({ w: Math.round(entry.contentRect.width), h: Math.round(entry.contentRect.height) }),
    );
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const running = workers.filter((w) => w.state === "running").length;
  const trunks = tasks === null ? [] : model(tasks, workers, minds);
  // The three rungs were three pills in the corner once, reading `mouth · brain · memory`.
  // They were this architecture's words for itself, parked where nothing explained them, and
  // they answered a question nobody asks. One of the states they carried is worth a person's
  // attention, and only when it is true — with no Reaction registered the agent cannot hear
  // you, however busy the chart looks.
  const deaf = tasks !== null && !workers.some((w) => w.role === "reaction");

  const wide = box.w >= 960;
  const geo = wide && trunks.length > 0 ? chart(trunks, box.w, box.h) : null;

  const root = (
    <div className="hi-home__root">
      <span className="hi-home__pip" data-live={running > 0 ? "true" : undefined} aria-hidden />
      {(tasks === null || deaf) && (
        <div className="hi-home__meta">
          {tasks === null ? <span>{L.reading}</span> : <span className="hi-home__deaf">{L.deaf}</span>}
        </div>
      )}
    </div>
  );

  return (
    <div className="hi-home" ref={frame}>
      <style>{CSS}</style>
      {geo ? (
        <Chart geo={geo} root={root} />
      ) : (
        <>
          {root}
          {trunks.length === 0
            ? tasks !== null && <p className="hi-home__empty">{L.nothingOpen}</p>
            : <Flow trunks={trunks} />}
        </>
      )}
    </div>
  );
}

// ── the chart ─────────────────────────────────────────────────────────────────

function Chart({ geo, root }) {
  const { hubX, hubY, width, height, arms } = geo;

  // One walk of an arm draws its wires, another its nodes. Both read the same laid-out
  // objects, so a label and the line reaching it cannot drift apart. `dir` is the only thing
  // that differs between the two arms.
  const wires = (nodes, arm, fromX, fromY) =>
    nodes.flatMap((node) => {
      const anonymous = node.label === null;
      const inX = anonymous ? fromX : node.x - arm.dir * 8;
      const outX = anonymous ? fromX : node.x + arm.dir * 8;
      const outY = anonymous ? fromY : node.y;
      return [
        anonymous ? null : (
          <path key={`${node.key}-in`} className="hi-home__wire" style={{ "--tone": TONE[node.tone] }}
            d={branch(fromX, fromY, inX, node.y)} />
        ),
        ...node.leaves.map((leaf) => (
          <path key={leaf.id} className="hi-home__wire hi-home__wire--leaf"
            style={{ "--tone": TONE[node.tone] }}
            d={branch(outX, outY, arm.leafX - arm.dir * 10, leaf.y)} />
        )),
        ...wires(node.children, arm, outX, outY),
      ];
    });

  const marks = (nodes, arm) =>
    nodes.flatMap((node) => [
      node.label === null ? null : (
        <div key={node.key} className="hi-home__trunk" data-dir={arm.dir > 0 ? "r" : "l"}
          data-rank={node.children.length ? "1" : "2"}
          style={{
            [arm.dir > 0 ? "left" : "right"]: arm.dir > 0 ? node.x : width - node.x,
            top: node.y,
            "--tone": TONE[node.tone],
          }}>
          <span className="hi-home__trunk-name">{node.label}</span>
          {node.count !== null && <span className="hi-home__trunk-n">{node.count}</span>}
        </div>
      ),
      ...node.leaves.map((leaf) => (
        <div key={leaf.id} className="hi-home__leaf" data-dir={arm.dir > 0 ? "r" : "l"}
          data-kind={leaf.kind || "task"} data-tone={leaf.tone}
          style={{
            [arm.dir > 0 ? "left" : "right"]: arm.dir > 0 ? arm.leafX : width - arm.leafX,
            top: leaf.y,
            maxWidth: arm.leafW,
            "--tone": TONE[leaf.tone],
          }}>
          <Leaf leaf={leaf} />
        </div>
      )),
      ...marks(node.children, arm),
    ]);

  return (
    <div className="hi-home__chart" style={{ height, "--scale": geo.scale }}>
      <div className="hi-home__rootnode" style={{ top: hubY, left: hubX }}>{root}</div>
      <svg className="hi-home__wires" width="100%" height={height} aria-hidden>
        {arms.map((arm) => (
          <g key={arm.dir}>{wires(arm.trunks, arm, hubX + arm.dir * 13, hubY)}</g>
        ))}
      </svg>
      {arms.flatMap((arm) => marks(arm.trunks, arm))}
    </div>
  );
}

/** Who is on the row, as a mark on the row rather than a rank of its own. Filled and
 *  breathing while a session is running, hollow while one is parked, absent when nobody is on
 *  it — and a numeral only when there is more than one, because "1" beside a dot is the dot
 *  said twice. */
function OnIt({ sessions }) {
  if (!sessions?.length) return null;
  const live = sessions.filter((worker) => worker.state === "running").length;
  return (
    <span className="hi-home__on" data-live={live > 0 ? "true" : undefined}
      title={sessions.map((w) => `${w.type || w.role} · ${w.state}`).join(", ")}>
      {sessions.length > 1 && <i>{sessions.length}</i>}
      <b aria-hidden />
    </span>
  );
}

/** The kind of session, when it is one that says something. `general` is the default type, so
 *  naming it tells a reader nothing; a builder or a reviewer being on the row is the half of
 *  the old pill worth keeping, and it belongs in the sentence rather than beside it. */
function kindOf(sessions) {
  const live = (sessions || []).find((worker) => worker.state === "running");
  const kind = live && (live.type || live.role);
  return kind && kind !== "general" ? L.doing[kind] || null : null;
}

function Leaf({ leaf }) {
  if (leaf.kind === "chips") {
    return (
      <div className="hi-home__chips">
        {leaf.chips.length === 0 ? (
          <span className="hi-home__note">{L.noMinds}</span>
        ) : (
          leaf.chips.map((mind) => (
            <span key={mind.subject} className="hi-home__chip">
              {mind.subject}
              {mind.at && <i>{L.since(ago(mind.at))}</i>}
            </span>
          ))
        )}
      </div>
    );
  }
  if (leaf.kind === "note") return <span className="hi-home__note">{leaf.title}</span>;
  const kind = kindOf(leaf.sessions);
  return (
    <>
      <span className="hi-home__head">
        <span className="hi-home__title" title={leaf.title}>{leaf.title}</span>
        {/* Topics this row is under that did not earn a rank of their own. */}
        {(leaf.marks || []).map((mark) => (
          <span key={mark} className="hi-home__mark">{mark}</span>
        ))}
        <OnIt sessions={leaf.sessions} />
      </span>
      {leaf.fact && (
        <span className="hi-home__fact">
          {/* Organised by state, the trunk said this. Organised by subject, nothing does —
              and a row whose blocker is the reader is the one thing that must not be found by
              reading every sentence on the page. */}
          {leaf.tone === "wait" && <b className="hi-home__you">{L.you}</b>}
          {kind && <em className="hi-home__kind">{kind}</em>}
          {leaf.fact}
        </span>
      )}
    </>
  );
}

// ── the narrow one ────────────────────────────────────────────────────────────
// A dendrogram needs width per rank and a phone has one rank's worth. Same model, same one
// fact, no geometry — the ranks become indentation.

function Flow({ trunks, rank = 1 }) {
  return (
    <div className="hi-home__flow" data-rank={rank}>
      {trunks.map((trunk) => (
        <section key={trunk.key} style={{ "--tone": TONE[trunk.tone] }}>
          {trunk.label !== null && (
            <h2>
              <span className="hi-home__trunk-name">{trunk.label}</span>
              {trunk.count !== null && <span className="hi-home__trunk-n">{trunk.count}</span>}
            </h2>
          )}
          {trunk.leaves.map((leaf) => (
            <div key={leaf.id} className="hi-home__leaf" data-kind={leaf.kind || "task"}
              data-tone={leaf.tone} style={{ "--tone": TONE[leaf.tone] }}>
              <Leaf leaf={leaf} />
            </div>
          ))}
          {trunk.children.length > 0 && <Flow trunks={trunk.children} rank={rank + 1} />}
        </section>
      ))}
    </div>
  );
}

// The only stylesheet in the file. A style prop cannot express a breakpoint, a pseudo-class
// or a descendant rule, and this surface wants all three; everything a prop *can* say that
// the geometry decides — a left, a top, a width — is still a prop, because it comes out of
// chart() and belongs to the node it positions.
//
// NOTE: no backticks anywhere below, not even inside a comment. One ends the template early
// and the file stops being JavaScript — see no_bundled_view_has_a_backtick_inside_its_css.
const CSS = `
.hi-home {
  position: relative;
  box-sizing: border-box;
  min-height: 100%;
  display: flex;
  flex-direction: column;
  padding: max(30px, var(--hi-safe-top)) clamp(18px, 3vw, 52px) 128px;
  color: var(--fg);
  font-family: var(--font-display);
}

/* ── the root ── */
/* On the chart it is a hub: a dot the branches come out of, parked on the vertical middle of
   everything hanging off it. In the flow it is the same dot at the top of the page. */
.hi-home__rootnode { position: absolute; transform: translate(-50%, -50%); }
.hi-home__root { display: flex; align-items: center; gap: 11px; }
.hi-home__chart .hi-home__root { flex-direction: column; gap: 7px; }
.hi-home__pip {
  flex: 0 0 auto;
  width: 15px;
  height: 15px;
  border-radius: 50%;
  background: var(--fg-mute);
  box-shadow: 0 0 0 6px color-mix(in srgb, var(--fg-mute) 9%, transparent);
}
.hi-home__pip[data-live] {
  background: var(--accent);
  animation: hi-home-pulse 2.6s var(--ease) infinite;
}
@keyframes hi-home-pulse {
  0%, 100% { box-shadow: 0 0 0 6px color-mix(in srgb, var(--accent) 12%, transparent); }
  50% { box-shadow: 0 0 0 13px color-mix(in srgb, var(--accent) 3%, transparent); }
}
/* Only ever an absence: reading, or unreachable. Never a summary of the branches. */
.hi-home__meta { white-space: nowrap; font-size: 12.5px; font-weight: 600; color: var(--fg-mute); }
.hi-home__deaf { color: var(--danger); font-weight: 700; }

.hi-home__empty { font-size: 14px; color: var(--fg-mute); margin: 26px 0 0; }

/* ── the chart ── */
/* Centred in whatever vertical room is left over, so a short map on a tall screen sits in the
   middle of it rather than in the top with the rest of the frame empty. A tall map fills the
   frame, the auto margins collapse, and it simply scrolls.
   The scale is the second lever: the map is laid out once and then opened out to whatever
   frame it was handed. A scale rather than a bigger type scale, because every proportion here
   was decided against the others. */
.hi-home__chart {
  position: relative;
  width: 100%;
  margin-block: auto;
  transform: scale(var(--scale, 1));
  transform-origin: center center;
}
.hi-home__wires { position: absolute; inset: 0; overflow: visible; pointer-events: none; }
.hi-home__wire {
  fill: none;
  stroke: var(--tone);
  stroke-width: 1.5;
  stroke-linecap: round;
  opacity: .34;
}
.hi-home__wire--leaf { stroke-width: 1.25; opacity: .22; }

.hi-home__trunk {
  position: absolute;
  transform: translateY(-50%);
  display: flex;
  align-items: baseline;
  gap: 7px;
  white-space: nowrap;
  /* On its own ground, because the wires to every other trunk pass behind this one and a
     branch drawn through a word reads as a strikethrough. */
  background: var(--paper);
  padding: 2px 7px;
  margin-left: -7px;
  border-radius: 5px;
}
/* The trunk label is not mirrored: reversed, KUT 4 reads 4 KUT. Only its anchor moves. */
.hi-home__trunk[data-dir="l"] { margin-left: 0; margin-right: -7px; }
.hi-home__trunk-name { font-size: 13px; font-weight: 800; letter-spacing: -.01em; color: var(--tone); }
.hi-home__trunk-n {
  font-family: var(--font-mono);
  font-size: 11px;
  font-weight: 700;
  color: var(--tone);
  opacity: .6;
}
/* A rank below a topic is a part of it, so it is quieter than the thing it is part of. */
.hi-home__trunk[data-rank="2"] .hi-home__trunk-name { font-size: 12px; font-weight: 700; opacity: .82; }

/* A leaf is a label on a rule, the way a branch ends on a mind map — not a card. Boxes were
   what made the first version of this page read as a board. It hugs its own content: stretched
   to the column measure it drew a full-width rule under two words. */
.hi-home__leaf {
  position: absolute;
  transform: translateY(-50%);
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding-bottom: 6px;
  border-bottom: 1px solid color-mix(in srgb, var(--tone) 22%, transparent);
  width: max-content;
  min-width: 0;
}
/* A row that wants somebody carries it on its own rule, since the trunk is a subject now and
   no longer says which pile a row is in. */
.hi-home__leaf[data-tone="wait"],
.hi-home__leaf[data-tone="warn"] {
  border-bottom-color: color-mix(in srgb, var(--tone) 55%, transparent);
}
/* The left arm is the same row read the other way. Right-aligned, NOT end-aligned: a column
   that shrink-wraps its children hands the title a content-sized box, and a title with no
   width to overflow does not ellipsise — it runs off the left edge of the window instead. */
.hi-home__leaf[data-dir="l"] { text-align: right; }
.hi-home__leaf[data-dir="l"] .hi-home__head { justify-content: flex-end; flex-direction: row-reverse; }

.hi-home__head { display: flex; align-items: center; gap: 8px; min-width: 0; }
.hi-home__title {
  font-size: 14px;
  font-weight: 700;
  letter-spacing: -.012em;
  color: var(--fg);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hi-home__fact {
  font-size: 11.5px;
  line-height: 1.35;
  color: var(--fg-mute);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hi-home__leaf[data-tone="wait"] .hi-home__fact { color: var(--danger); }
.hi-home__you {
  display: inline-block;
  margin-right: 7px;
  padding: 0 6px;
  border-radius: 999px;
  border: 1px solid var(--danger-line);
  background: var(--danger-wash);
  font-size: 10px;
  font-weight: 800;
  letter-spacing: .05em;
  text-transform: uppercase;
  vertical-align: 1px;
}
/* What kind of session, when the kind says something. Reads as the opening of the sentence it
   is part of, which is why it is not a chip. */
.hi-home__kind { font-style: normal; font-weight: 700; color: var(--accent); margin-right: 6px; }
/* A topic this row is under that did not earn a rank of its own. */
.hi-home__mark {
  flex: 0 0 auto;
  font-size: 10.5px;
  font-weight: 700;
  color: var(--fg-mute);
  border: 1px solid var(--surface-border);
  background: var(--surface);
  border-radius: 4px;
  padding: 1px 6px;
}
/* Somebody is on this row. It follows the title rather than parking at the end of the rule:
   the rule runs to the end of the row and the words often do not, so a dot out there was a
   detail about the row sitting nowhere near it. */
.hi-home__on { flex: 0 0 auto; display: flex; align-items: center; gap: 4px; }
.hi-home__on i {
  font-style: normal;
  font-family: var(--font-mono);
  font-size: 10px;
  font-weight: 700;
  color: var(--fg-mute);
}
.hi-home__on b {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  border: 1.5px solid var(--line-strong);
  box-sizing: border-box;
}
.hi-home__on[data-live] i { color: var(--accent); }
.hi-home__on[data-live] b {
  border-color: var(--accent);
  background: var(--accent);
  animation: hi-home-on 2.2s var(--ease) infinite;
}
@keyframes hi-home-on {
  0%, 100% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--accent) 34%, transparent); }
  60% { box-shadow: 0 0 0 5px color-mix(in srgb, var(--accent) 0%, transparent); }
}

.hi-home__note { font-size: 12.5px; color: var(--fg-mute); }
.hi-home__chips { display: flex; gap: 6px; flex-wrap: wrap; }
.hi-home__chip {
  display: inline-flex;
  align-items: baseline;
  gap: 5px;
  padding: 2px 9px;
  border-radius: 999px;
  border: 1px solid var(--surface-border);
  background: var(--surface);
  font-size: 11.5px;
  font-weight: 650;
  color: var(--fg-dim);
}
.hi-home__chip i {
  font-style: normal;
  font-family: var(--font-mono);
  font-size: 10.5px;
  color: var(--fg-mute);
}

/* ── the narrow one ── */
.hi-home__flow { margin-top: 22px; display: flex; flex-direction: column; gap: 20px; }
.hi-home__flow[data-rank="2"] { margin-top: 12px; margin-left: 16px; gap: 14px; }
.hi-home__flow[data-rank="2"] h2 .hi-home__trunk-name { font-size: 12px; }
.hi-home__flow h2 { margin: 0 0 8px; display: flex; align-items: baseline; gap: 7px; }
/* The chart hands each leaf a max-width out of the geometry; the flow has no geometry, so the
   row that hugs its content there has to be told not to hug past the screen. */
.hi-home__flow .hi-home__leaf {
  position: relative;
  transform: none;
  width: auto;
  max-width: 100%;
  margin-left: 14px;
  padding: 0 0 7px;
  margin-bottom: 9px;
}
`;
