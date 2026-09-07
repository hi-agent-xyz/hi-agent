// purpose: 现在 — the agent's open work drawn as one chart: what it owes, grouped by what
// the work is about, with who is on each thing and what it has had its head in, laid out
// around a single hub. The surface for when nothing more specific is up, which is most of the time.
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
//                  ┌──────────────┐   ⟨没有项目 5⟩ ─┐        ┌─ ⟨songguo⟩ manage auto-deployer
//     ⟨hi-agent 3⟩ ─┤ [缩略图]      │        │        │             值守 · 2 小时前确认还活着
//                  │ 制作 20 周…   │   ▪ 生词本 ▪周报  │        │
//                  │ 等你 · 打开 → │        │        ●        └─ ⟨KUT 3⟩ deploy KUT ⟨gz⟩
//                  └──────────────┘   ⟨还想着⟩ ──────┘  hub          没人管 · 已经放了 3 天
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
// already in the data: **B is inside A when every row on the chart carrying B also carries A,
// and A holds strictly more rows.** `Tencent COS` appears only on rows that are also `KUT`, so it
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
// (`docs/arch/data.md`). And a row tagged with nothing is still not filed under "misc",
// which would claim a kinship the store does not record — it goes under **no project**,
// which claims the opposite and is what the record actually says. That branch is not a
// subject; it is the chart's word for the absence of one, in the family of `closed` and
// `also on its mind`. It used to have no branch at all and hang straight off the hub, and
// the cost was the chart's whole point: five rows arriving in the same leaf column as the
// filed ones, from the same point, with nothing on the screen saying which was which.
//
// ── what is drawn ───────────────────────────────────────────────────────────
//
// **Three edges, and every one is stored.** `task ← session` is the worker's `subject`,
// which `CreateWorker` requires and refuses when nothing is filed under it. `session ←
// session` is `owner`, an id. `task → view` is the record's own inline-code tokens, checked
// against `/api/views` so a token is a view only when that view exists. Nothing here is
// inferred and nothing is drawn that a field does not say.
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
// **One fact per leaf**, in the order `tasks.jsx` arrived at for its cards: the `waiting` line
// if a human is the blocker; else what the session on it says it is doing and how long
// since; else **nobody on it** and how long it has stood there; else last-confirmed-alive for
// a duty; else age. The third is the reason to build this — nothing on any current surface
// says it, and a restart's casualties are not in the roster at all, so a row whose worker
// died reads as work in progress on the board and as nothing anywhere else.
//
// Both of the first two are prose written for somewhere else, so both are cut to one line by
// `oneLine` before they land: a `waiting` note is a record entry carrying its refs and its
// caveats, and a session's title is a sentence from the roster. The cut only ever removes.
//
// **A card is a control when the row names a view**, which on the live store is more than half
// of the open ledger. The ref is already written into the record — `zhao-li-checkin/…` sits in
// the very sentence asking for the review — `/api/views` already says which refs exist and
// hands back the picture the host already took, and `useViews().openRef` already puts the
// screen on one. Nothing was added to the wire for this. What this file used to say could not
// be done is a different thing that still cannot: opening `factory/tasks` *at a given row*,
// which needs one optional field on `POST /api/views/open`.
//
// **Four shapes, told apart by what goes in them and not by what state the row is in.** A
// picture card, a text card, a row, a chip; every member of a class is the same size, which is
// what makes a wall of them read as tidy. Classing them by state instead put a row that wants a
// person but has no picture into the picture card's height — a box of mostly nothing with the
// title pressed against the bottom edge, which reads as a view that failed to render. A size
// class is only tidy when its members hold the same kind of thing.
//
// **The picture is never cropped, and the shapes are not uniform.** The same view re-shot in a
// narrow window comes back portrait: 393×852 against the usual 479×271. The card is a fixed
// size, so a tall picture has to leave room at its sides; that room is a mat and the picture
// carries its own edge and shadow, which is what makes it read as a page rather than a fault.
//
// **Who is on a row is a mark on the row**, not a rank of its own. It was a fourth rank of
// pills once: a column of chips floating in the right of the window, wired back to their
// rows because nothing else said whose they were, saying `general running` beside a sentence
// that already read `pytest -k live · 1m`. What the pills alone knew was *how many* and
// *what kind* — a numeral and a word, both of which fit on the row.
//
// ── the frame ───────────────────────────────────────────────────────────────
//
// **Lanes, because a canvas that is only relaxed is a canvas nobody can read.** Two earlier
// cuts placed the topics by physics — seed them on an ellipse, push overlapping boxes apart,
// stop. Both looked scattered, and the reason is not that the solver was weak: it had no
// objective at all. It halts the moment nothing overlaps, so every cluster ends up at whatever
// coordinate one particular iteration produced and no two gaps are the same. The canvases that
// read as deliberate are not better simulated; **their contents share edges**. So the placement
// is a small number of equal-width lanes with the hub at the centre of the middle one, and each
// topic goes whole into the lane where it costs least.
//
// **The hub is not special-cased**: it is a fixed box that takes part in the packing, so the
// clearing in the middle is space it occupies rather than space reserved for it.
//
// **What is optimised is how big everything ends up.** The lane count is chosen by running the
// packing at 3, 5 and 7 and keeping whichever yields the largest final scale — odd only, since
// an even count puts the centre line between two lanes and leaves the hub without one. An
// earlier cut optimised for a canvas whose proportions matched the frame, which is a proxy and
// pointed the wrong way: it chose five lanes, five lanes exactly filled the width, the scale was
// pinned at 1 and a third of the height went empty. Once the count is settled, the width still
// spare goes into the gutters — into air, never into bigger cards, because a card's size is
// decided by what it holds and must not drift with the window.
//
// **A wire travels in a gutter**, because a gutter is the only space the packing guarantees is
// empty — it keeps the lanes from overlapping each other and says nothing about what stands
// between a wire's two ends. Leaving the hub radially, straight at the cluster it is going to,
// crosses whatever lane is in between by construction: measured on the store this was designed
// against, the wire to `songguo` spent a third of its length inside the `also on its mind`
// cluster and the one to `hi-agent` clipped the same chips from the other side. So a
// side-entering wire leaves the hub level, turns up or down the gutter beside its lane, and
// comes in level with the label. The heading's paper ground went with the crossings it was
// there for: painted in a 30px box, a gradient starts over, so it was a lighter chip laid on
// the page in every skin whose two paper stops differ.
//
// **The canvas is whatever the content grew to, and the frame then scales it to fit.** That
// order is the whole idea; sizing the layout to the frame first is what produced a sparse ring
// with a hole in it.
//
// Below 960px wide (or 520px tall) it stops being a canvas — lanes need width and a phone has
// one lane's worth — and the same model renders as sections down the page, same four shapes,
// same thumbnails, same controls, no geometry.
//
// **Twelve branches, not a hundred and forty-nine.** The store this was designed against
// carries 149 rows — 3 todo, 5 doing, 4 serving, 108 done, 25 cancelled — so 8% of the ledger
// is open. That number is the layout. Closed work is one chip carrying two counts;
// `tasks.jsx` already learned the expensive version of this when five equal columns spent 40%
// of the width on the 93% of rows that had closed.
//
// **But a row does not leave at the instant it closes, and that is not a softening of the
// number above.** The moment a thing is finished is the moment it is most worth looking at:
// the work is done, the deliverable exists, and nobody has seen it yet. Dropping the row on
// the status change spent that moment saying nothing — on the live store the sports-AI
// research closed at 20:05 and by 22:45 this surface held its name nowhere, its picture
// nowhere, and `+1` inside `6 today`, which reads as *it did not happen*. So a `done` row
// that left a view keeps its place for a while: same tags, same `forest`, same topic, one
// rank cooler. `stillStanding` in the model section is the whole of it — what leaves is
// decided by displacement first and a clock second, and none of it is a branch of its own.
//
// **What this cannot do yet, in the file rather than after the fact:**
//   - The project trunk costs one request per project, because `GET /api/facets` returns
//     sorted names and no timestamps. It reads on attention rather than on a clock for
//     exactly that reason. A `modified` on the list endpoint removes it.
//   - A card opens the view a row is *about*; nothing opens the row itself. That needs the
//     optional field on `POST /api/views/open` described above, and it is general to every
//     pair of factory views rather than anything this page should carry alone.
//   - Its tile in the band is a picture of this file's loading state. `settle()` in the
//     render page waits two frames, fonts and images, and not for a fetch, so the shot is
//     taken before the first read comes back. That is every data-driven surface here
//     (`_shots/ref/factory/tasks.png` is an empty board reading "0 active"), and it lands
//     hardest on this one, which is the surface a person is most likely to reach for cold.
//   - It is not walkable on a television. Spatial navigation runs in the host's cover plane;
//     a view's own arrows are its own and this view binds none. The cards are now real buttons,
//     so a D-pad has something to land on the day that plane reaches a view — but nothing has
//     been read from across a room.
//   - The four corners still carry air, and the lane count is coarse: 3, 5, 7 and nothing
//     between. Both are quantities to tune, not structure to redo.
//   - **A wire to a cluster two lanes out still crosses the lane in between.** Its gutter is
//     beside its own lane, and reaching it means passing the inner one at hub height — where
//     that lane centres its stack, so something is nearly always there. Over 600 random stores
//     that is the whole of what is left: three lanes went from 184 crossings to 19, five lanes
//     from 1292 to 1153. Fixing it wants a route with two turns rather than one, or a packing
//     that knows wires exist; neither is worth it while the live store lays out in three.
//   - **Nothing in the host decides when it goes up**, and nothing should: `stage.md` refuses
//     a host gate on what is on the screen. It is a view like any other — reachable from the
//     bookmarks row, shown by `hi_show`. When Reaction reaches for it is guidance, and it
//     lives in `identity/reaction.md` beside the rule it completes: *an old answer left
//     standing says you are still on it*, which until now had only two endings and both were
//     bad — leave the stale view up, or dismiss to an empty room. It is where the screen
//     **rests**: up when the subject is the state of the work rather than any one piece of
//     it, and replaced the moment a particular thing exists to put there.
//
// Colour comes from the host theme tokens (see tasks.jsx for the vocabulary). Type is three
// sizes and two weights for the whole surface — `TYPE` in the canvas section, which the
// stylesheet reads back out as custom properties and the chip ruler measures in. It had seven
// sizes and four weights, which is not a hierarchy but a hierarchy per element.
import { useCallback, useEffect, useRef, useState } from "react";
import { useLive, useViews, TEMPO } from "@hi/core";

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
    trunk: { loose: "no project", minds: "also on its mind", closed: "closed" },
    // One word per row, in the row's own tone. It replaces the four places the old surface
    // said the same thing (red title, red badge, red rule, red trunk name).
    state: {
      wait: "you",
      warn: "nobody",
      live: "running",
      serving: "on duty",
      todo: "parked",
      done: "done",
    },
    open: "open",
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
    trunk: { loose: "没有项目", minds: "还想着", closed: "已结束" },
    state: { wait: "等你", warn: "没人管", live: "在跑", serving: "值守", todo: "停着", done: "完成" },
    open: "打开",
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
// **How a finished row leaves: pushed, or aged out — never the instant it closes.** Two
// forces, because either alone is wrong. A cap alone would keep the last delivery up for a
// week on a quiet stretch; a clock alone would hold four cards through a busy afternoon and
// bury the open work under what is already done. So the standing set is the newest
// `STANDING_SHOWN` deliveries inside `STANDING_HOURS`, and a fourth delivery displaces the
// oldest of the three the moment it lands.
const STANDING_SHOWN = 3;
const STANDING_HOURS = 24;
// A ceiling on how far down the project list the row will look. It used to bound the number
// of *requests* — one per project — and now bounds only how much of one response is read.
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
const HEAT = ["wait", "warn", "live", "serving", "todo", "done", "minds", "closed"];
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
  // Names *and* mtimes, so what a project was last thought about is one request rather
  // than one per project. That is the whole reason this row can be on a clock.
  facets: () => fetch("/api/facets").then((r) => r.json()),
  // Names every view that exists, with the picture already taken for the ones that have one.
  views: () => fetch("/api/views").then((r) => r.json()),
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

// **A leaf holds one line; the record holds the paragraph.** Both strings that reach a leaf
// arrive written for somewhere else. A `waiting` line is a note the mind wrote into an
// append-only record, so it carries the whole ask with its refs and its caveats — on the live
// store, `请当前 unknown sender 在 hi-agent 已显示的 screen slot ...（stable ref ...）审阅当前纯展示
// 候选，并回复 **ACCEPT**、**REJECT** 或具体修改；独立 PASS、...` in one leaf. A session `title`
// is short already but is still prose from another surface.
//
// So the same reduction runs on both, and it only ever *removes*: markdown emphasis, the
// backticks around a ref (the ref itself is the fact and stays), parenthetical asides, opaque
// ids, and everything after the first sentence. An aside is by construction the part the
// sentence works without; an id is the part nobody can read and nobody can act on from across
// a room. Both are in these strings because the record is addressed to whoever opens the row
// next, and this line is addressed to whoever is looking at the wall — and neither is lost,
// because the row still holds the whole thing. What survives is the first thing the record
// says, which is what the row is about; the rest is a paragraph, and a paragraph on a leaf
// ellipsises mid-clause and says nothing at all.
const ASIDE = /\s*[（(][^）)]*[）)]/g;
const OPAQUE = /\s*\b[0-9a-f]{8}-[0-9a-f-]{8,}\b/gi;
function oneLine(text) {
  let out = String(text || "").trim();
  out = out.replace(/\*\*(.+?)\*\*/g, "$1").replace(/`([^`]+)`/g, "$1");
  out = out.replace(ASIDE, "").replace(OPAQUE, "");
  // The first sentence, by the punctuation either language ends one with. A string that never
  // terminates is taken whole and left to the row to clip.
  const cut = out.search(/[。！？；;!?]|\. /);
  if (cut > 0) out = out.slice(0, cut);
  return out.trim();
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
  // **A finished row's one fact is when**, and the card's picture is the rest of it.
  //
  // The summary sentence this line ought to carry has a place in the record already, and the
  // instruction for it is right: a `delivered` entry is "the person has something now, or it
  // went out" (`identity/workers/general.md`), with `digest posts at 09:00; today's is in the
  // group as om_xxx` as the shape. On the live store neither standing row had written one.
  // Both had put the audit there instead — *Reaction 已 exact show stable ref … Show 后读取
  // /api/out/view version 4041* — prose addressed to whoever would later check that the show
  // really happened, which is true, and is not a thing to hand a person under a picture.
  //
  // So this line does not gamble on it. Nothing in a view can tell a sentence from a receipt,
  // and the failure is not symmetric: a missing summary costs a line the picture mostly
  // covers, while a receipt printed in the card's one sentence reads as the surface being
  // broken. If that entry becomes reliably a sentence, this is where it goes — that is a
  // question about what cognition writes when it delivers, and it is measured in the record,
  // not repaired here.
  if (task.status === "done") {
    const age = ago(task.completedAt || task.statusSince);
    return { tone: "done", text: age ? L.since(age) : "" };
  }

  const wait = waitsOnPerson(task) ? latestSpoken(task) : null;
  if (wait) return { tone: "wait", text: oneLine(wait.text) };

  const live = (crew || []).find((w) => w.state === "running");
  if (live) {
    // **The session's `title`, not its `doing`.** `doing` is the harness's word for the turn
    // it is in the middle of, and on the live store it is one of two useless things: a state
    // with no content (`thinking`, on four of the nine sessions at once) or the shell call
    // verbatim (`$ /bin/zsh -lc "shasum -a 256 \\…`). A pass that stripped the wrapper and
    // took the first line still left `control_pid=$$; printf ...` on the row — the packaging
    // was never the problem, a tool call is simply not what the work is.
    //
    // `title` is: one line the session wrote about what it is doing on this row, which is the
    // fact this leaf was always supposed to carry. It does not tick, and nothing is lost by
    // that — the age beside it does, and the dot on the row is already what says *moving*.
    const age = ago(live.doing_at || live.state_since);
    const said = oneLine(live.title);
    return { tone: "live", text: said ? `${said}${age ? ` · ${age}` : ""}` : "" };
  }

  if (task.status === "serving") {
    const age = ago(task.checkedAt);
    // A duty that has never reported is the one `serving` row that is an alarm.
    return age ? { tone: "serving", text: L.alive(age) } : { tone: "warn", text: L.neverChecked };
  }

  if (task.status === "doing") {
    const age = ago(task.statusSince);
    // The row already carries **nobody** as its state word, so the fact is only how long.
    return { tone: "warn", text: age ? L.stood(age) : L.nobody };
  }

  const age = ago(task.statusSince || task.createdAt);
  return { tone: "todo", text: age ? L.since(age) : "" };
}

// **The view a row is about, which the record already names.** A deliverable is written into
// the record as an inline-code ref the moment it exists — `zhao-li-checkin/zhao-li-20-week-checkin`
// in the very sentence that asks for the review — so nothing here is inferred: a token is a view
// only when `/api/views` says that view exists, and the picture beside it is the one the host
// already took. Newest entry first, because a row that has been through three deliverables is
// about the last one.
//
// `factory/*` is excluded. Those are this system's own boards; a record names them as evidence
// ("checked it against the tasks board"), never as the thing the row is about, and on the live
// store that is exactly how a review row picked up `factory/tasks` instead of its own candidate.
const REF = /`([a-z0-9][a-z0-9-]*\/[a-z0-9][a-z0-9-]*)`/g;
function viewRefOf(task, known) {
  if (known.size === 0) return null;
  const said = [];
  const timeline = task.timeline || [];
  for (let i = timeline.length - 1; i >= 0; i -= 1) if (timeline[i].text) said.push(timeline[i].text);
  if (task.body) said.push(task.body);
  for (const text of said) {
    for (const hit of String(text).matchAll(REF)) {
      if (hit[1].startsWith("factory/")) continue;
      const view = known.get(hit[1]);
      if (view) return view;
    }
  }
  return null;
}

/** How much has closed, over a day and over a week.
 *
 *  **The rows [`stillStanding`] kept are counted here too**, so a chart showing two finished
 *  cards still says `6 today` rather than `4 more today`. The count is a count of what closed,
 *  which is a fact about the ledger and not about this chart's spare width — subtracting what
 *  happens to be drawn would make the same day read as a different number depending on how
 *  tall the window is. */
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

/** **What has just been delivered, still standing where it was done.**
 *
 *  A row used to leave the chart at the instant its status changed, which is the one moment
 *  it is most worth looking at: the work is finished, the thing exists, and nobody has seen
 *  it yet. On the live store the sports-AI research closed at 20:05, was delivered as a view
 *  whose picture had already been taken, and by 22:45 the surface built to answer *where is
 *  everything* held its name nowhere and its picture nowhere — `+1` inside `6 today`. The
 *  reading that produces is that the work did not happen.
 *
 *  **`done`, and a deliverable.** Cancelled is not finished, and a card headed 完成 over a row
 *  that was abandoned is a lie the chart would be telling in its largest type. And the shape a
 *  finished row gets is the picture card, so a row with nothing to open has nothing to put in
 *  it — the picture *is* what keeps it here — and it goes to the count as before. What stands
 *  is what a person would ask about by name.
 *
 *  **It stands where it was done, not in an archive.** The row keeps its tags, so it hangs off
 *  its own topic through the same `forest` every other row goes through; a finished KNQ map
 *  files under `ktv` because that is what its record says, and a row that carried no tag lands
 *  under `no project` for the same reason it would have while open. Nothing here places
 *  anything — that is the point of not building a separate branch for it. */
function stillStanding(tasks, known, now) {
  const out = [];
  for (const task of tasks) {
    if (task.status !== "done") continue;
    const at = stamp(task.completedAt || task.statusSince);
    if (!at) continue;
    if ((now - at.getTime()) / 3600000 > STANDING_HOURS) continue;
    const view = viewRefOf(task, known);
    if (!view) continue;
    out.push({ at, task, view });
  }
  out.sort((a, b) => b.at.getTime() - a.at.getTime());
  return out.slice(0, STANDING_SHOWN);
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

/** Fold the tags of every row *on the chart* into a forest of topics and say which node each
 *  row hangs off. Returns plain data; nothing here knows the chart exists.
 *
 *  "On the chart" is the open ledger plus what [`standing`] kept, and the containment rule
 *  reads over the same set: a topic that exists only because of a delivery that finished this
 *  morning is a topic the work really has, and the alternative — computing the shape from the
 *  open rows and then hanging finished ones off it — would file a row under a node derived
 *  from a set that row is not in. */
function forest(shown, projectNames) {
  const rows = new Map(); // folded tag -> { label, ids:Set }
  const carried = new Map(); // task subject -> folded[]
  let tagged = 0;

  for (const { task } of shown) {
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
function model(tasks, workers, minds, views) {
  const crew = crewBySubject(workers);
  const shown = [];
  const put = (task, view) => {
    const on = crew.get(task.subject) || [];
    const one = read(task, on);
    shown.push({
      task,
      leaf: {
        id: task.subject,
        title: task.title || task.subject,
        fact: one.text,
        tone: one.tone,
        sessions: on,
        age: ago(task.statusSince || task.createdAt),
        view: view === undefined ? viewRefOf(task, views) : view,
      },
    });
  };
  for (const task of tasks) if (OPEN.has(task.status)) put(task);
  // Its view is already resolved, so it is handed over rather than looked up twice.
  for (const row of stillStanding(tasks, views, Date.now())) put(row.task, row.view);

  const projectNames = new Set(minds.map((m) => m.subject));
  const { rows, parent, home } = forest(shown, projectNames);

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
  for (const row of shown) {
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
    // **These used to hang straight off the hub, unlabelled, and that was the chart's worst
    // reading.** The refusal behind it is still right — filing them under "misc" would claim a
    // kinship the store does not record — but what it produced was five rows arriving in the
    // same leaf column as the filed ones, from the same point, down wires nobody can trace. On
    // the live store you could not tell which row was songguo's and which was under nothing,
    // which is the one question a chart organised by subject exists to answer.
    //
    // A junction of their own fixes it and invents nothing, because the label is not a subject:
    // it is the chart's own word for the absence, in the same family as `closed` and `also on
    // its mind`. It says these rows carry no topic, which is exactly what the record says.
    roots.push({
      key: "__loose",
      label: L.trunk.loose,
      count: loose.length,
      tone: loose.map((l) => l.tone).reduce(hotter, "closed"),
      children: [],
      leaves: loose,
    });
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

// ── the canvas ────────────────────────────────────────────────────────────────
// Nothing here but numbers. Every cluster position, every wire endpoint and every card height
// is read off what this returns, so a label and the line reaching it cannot drift apart.

const W = 320;
// **Four shapes, and they are told apart by what goes in them, not by what state the row is in.**
// An earlier cut classed them by state, so a row that wants a person but has no picture got the
// picture card's height — a 320x232 box holding three lines of grey text with the title pressed
// against the bottom edge, which reads as a view that failed to render. A size class is only
// tidy when its members hold the same kind of thing; otherwise uniform size is uniform emptiness.
const BOX = { P: 232, T: 128, R: 60 }; // picture card / text card / one row
const LABEL_H = 30;
const SUB_H = 24;
// **Two gaps, because they say different things.** Between the cards inside a cluster, and
// between two clusters sharing a lane. One constant doing both jobs makes them the same width,
// and since a card has no outer frame, that space is the only thing on the surface saying
// *these are two separate things* — so the grouping stops reading at all.
const GAP = 9;
const LANE_GAP = 46;
// A chip is a fixed height in the stylesheet rather than whatever its contents come to, so the
// row arithmetic below is exact instead of nearly right — and so a run of them reads as a run.
const CHIP_H = 24;
const CHIP_GAP = 6;
const GUT = 34;
const PAD = 34;
// The hub is not special-cased in the packing: it is a fixed box that takes part in it, so the
// clearing in the middle is space it *occupies* rather than space left for it.
const HUB_W = 210;
const HUB_H = 118;
// Where a wire starts: clear of the pip and the ring around it, so a line grows out of the hub
// rather than out from under it.
const HUB_R = 13;

// **A finished row is a picture card, and never anything else.** `standing` only keeps rows
// that left a view, so the `P` here has no fallback by construction — the picture *is* the
// reason the row is still on the chart. It is the same shape a row waiting on a person gets,
// which is right: both are rows whose whole content is *there is a thing here, look at it*.
const shapeOf = (leaf) =>
  leaf.tone === "todo"
    ? "XS"
    : leaf.tone === "done"
      ? "P"
      : leaf.tone === "wait"
        ? leaf.view
          ? "P"
          : "T"
        : "R";

// **How wide a chip will be, measured rather than counted.** It was `title.length * 7.4`, which
// is wrong by a whole row the moment a title is CJK — one em a character, not half — and it
// left out the timestamp beside the name entirely, which is most of a chip's width. A row out is
// a cluster 30px shorter than the one on the screen, and on a cluster whose label sits at the
// bottom that ended the wire above its own name, leaving a stub of line over the word.
//
// `measureText` is the same engine that will lay the chip out, so it cannot disagree with it.
// The ruler is made once, and only where there is a document; without one the old character
// count is the fallback. A read taken before the display face has finished loading measures the
// fallback face, and the next one — the ledger reads every few seconds — corrects it.
// **Three sizes for the whole surface**, in JavaScript rather than in the stylesheet because
// the ruler has to measure in the same type the chip is set in. The stylesheet reads them back
// out as custom properties, so there is still one place to change them.
const TYPE = { lg: 15, md: 12, sm: 10.5 };
const CHIP_PAD = 20; // 9px of padding either side, plus the hairline
const CHIP_THUMB = 31; // the thumbnail and the gap before it
const CHIP_GAP_IN = 5; // between a chip's own parts
let ruler = null;
let fonts = null;
function faces() {
  if (fonts) return fonts;
  const root = typeof document !== "undefined" && document.documentElement;
  const read = (name) => (root ? getComputedStyle(root).getPropertyValue(name).trim() : "");
  fonts = {
    chip: `700 ${TYPE.md}px ${read("--font-display") || "sans-serif"}`,
    mark: `700 ${TYPE.sm}px ${read("--font-mono") || "monospace"}`,
  };
  return fonts;
}
function textWidth(text, font) {
  if (ruler === null) {
    ruler =
      typeof document !== "undefined" && document.createElement("canvas").getContext
        ? document.createElement("canvas").getContext("2d")
        : false;
  }
  if (!ruler) return String(text).length * 7.4;
  ruler.font = font;
  return ruler.measureText(String(text)).width;
}
function chipWidth(chip) {
  const face = faces();
  return (
    CHIP_PAD +
    textWidth(chip.title, face.chip) +
    (chip.fact ? CHIP_GAP_IN + textWidth(chip.fact, face.mark) : 0) +
    (chip.view ? CHIP_THUMB : 0)
  );
}
function chipRows(list) {
  let rows = 1;
  let x = 0;
  for (const chip of list) {
    const w = chipWidth(chip);
    if (x > 0 && x + w > W) {
      rows += 1;
      x = 0;
    }
    x += w + CHIP_GAP;
  }
  return rows;
}

/** A topic, flattened into the blocks that draw it: its cards top to bottom, a small heading
 *  wherever a sub-topic earned a rank, and everything parked collected into one run of chips at
 *  the foot. **A cluster is one column of fixed width**, which is what makes the packing a
 *  one-dimensional problem and what lets anything line up at all. */
function cluster(node) {
  const blocks = [];
  const chips = [];
  const walk = (n, sub) => {
    if (sub) blocks.push({ t: "sub", id: `sub-${n.key}`, label: n.label, count: n.count, h: SUB_H });
    for (const leaf of n.leaves) {
      if (leaf.kind === "chips") {
        for (const mind of leaf.chips) {
          chips.push({ id: mind.subject, title: mind.subject, fact: mind.at ? L.since(ago(mind.at)) : null });
        }
        continue;
      }
      if (leaf.kind === "note") {
        chips.push({ id: leaf.id, title: leaf.title });
        continue;
      }
      const k = shapeOf(leaf);
      if (k === "XS") chips.push({ id: leaf.id, title: leaf.title, view: leaf.view, leaf });
      else blocks.push({ t: "card", id: leaf.id, leaf, k, h: BOX[k] });
    }
    for (const child of n.children) walk(child, true);
  };
  walk(node, false);

  // **The height the browser will stack, not an estimate of it.** The column is the label, each
  // block, and the chip run, with one GAP between every pair. The old sum charged a gap to every
  // block and none to the chips, so a cluster stood 4 to 9px taller than the layout believed —
  // and on a cluster whose label is at the *bottom* that put the wire's endpoint above its own
  // label, leaving a stub of line poking out over the word. Every part of the column is now a
  // fixed height in the stylesheet, which is what lets this be arithmetic rather than a guess.
  const parts = [LABEL_H, ...blocks.map((b) => b.h)];
  if (chips.length > 0) {
    const rows = chipRows(chips);
    parts.push(rows * CHIP_H + (rows - 1) * CHIP_GAP);
  }
  const h = parts.reduce((n, x) => n + x, 0) + (parts.length - 1) * GAP;
  return { key: node.key, label: node.label, tone: node.tone, count: node.count, blocks, chips, w: W, h: Math.max(h, LABEL_H) };
}

/** Deal the clusters onto lanes. Each one goes whole into the lane where it costs least:
 *  **the extent it adds beyond the centre line**, plus a small premium for sitting further out
 *  so the hot ones land near the hub.
 *
 *  The extent is the point. A lane that is not the hub's is centred as a whole, so a cluster
 *  there reaches half its height either way; the hub's lane grows from the middle block outward,
 *  so a cluster there reaches its full height. Comparing a whole stack against a one-way reach
 *  makes the hub lane look half as tall as it is, and it becomes a magnet — on the live store
 *  five of seven clusters fell into it, the canvas grew to 974px, and the scorer had to retreat
 *  to more lanes and spread everything into one flat band. */
function assign(cs, C) {
  const m = (C - 1) / 2;
  const lanes = Array.from({ length: C }, (_, i) => ({ i, up: [], down: [], hub: i === m }));
  const reach = (lane, dir) => {
    let n = lane.hub ? HUB_H / 2 : 0;
    for (const c of lane[dir]) n += c.h + LANE_GAP;
    return n;
  };
  for (const c of cs) {
    let pick = null;
    for (const lane of lanes) {
      // Only the hub's lane has two directions. Treating an ordinary lane as two stacks here and
      // one stack when drawing makes its second cluster look free, so a tall cluster always ends
      // up with a passenger while other lanes stand empty.
      for (const dir of lane.hub ? ["up", "down"] : ["down"]) {
        const after = reach(lane, dir) + c.h + (lane[dir].length || lane.hub ? LANE_GAP : 0);
        const cost = (lane.hub ? after : after / 2) + Math.abs(lane.i - m) * 30;
        if (!pick || cost < pick.cost) pick = { lane, dir, cost };
      }
    }
    pick.lane[pick.dir].push(c);
  }
  return { C, m, lanes };
}

/** Lay one lane plan out in coordinates whose origin is the hub. */
function put({ m, lanes }, gut) {
  for (const lane of lanes) {
    const cx = (lane.i - m) * (W + gut);
    if (lane.hub) {
      let y = -HUB_H / 2;
      lane.up.forEach((c, i) => {
        y -= LANE_GAP + c.h;
        c.cx = cx;
        c.cy = y + c.h / 2;
        c.hubLane = true;
        c.slot = i;
      });
      y = HUB_H / 2;
      lane.down.forEach((c, i) => {
        c.cx = cx;
        c.cy = y + LANE_GAP + c.h / 2;
        y += LANE_GAP + c.h;
        c.hubLane = true;
        c.slot = i;
      });
    } else {
      const all = [...lane.up.slice().reverse(), ...lane.down];
      const h = all.reduce((n, c) => n + c.h + LANE_GAP, -LANE_GAP);
      let y = -h / 2;
      for (const c of all) {
        c.cx = cx;
        c.cy = y + c.h / 2;
        y += c.h + LANE_GAP;
        c.hubLane = false;
        c.slot = 0;
      }
    }
  }
  // **The x a wire runs along on its way here.** The packing guarantees exactly one kind of
  // empty space — the column between two lanes — so that is where a wire travels: the gutter
  // between this cluster's lane and the hub's. A cluster sharing the hub's lane and coming in
  // from the side goes around through the gutter beside it, which is the same rule seen from
  // the inside; one straight above or below the hub has nothing to go around and runs down the
  // hub's own line.
  const half = (W + gut) / 2;
  for (const lane of lanes) {
    for (const c of [...lane.up, ...lane.down]) {
      const a = anchorOf(c);
      if (a === "top" || a === "bottom") c.gx = c.cx;
      else if (c.cx) c.gx = c.cx - Math.sign(c.cx) * half;
      else c.gx = (a === "right" ? 1 : -1) * half;
    }
  }
}

/** Choose the lane count, then spend what is left over on air.
 *
 *  **The thing being optimised is how big everything ends up**, so that is what is measured. An
 *  earlier cut optimised for a canvas whose proportions matched the frame, which is a proxy, and
 *  it pointed the wrong way: it chose five lanes, five lanes exactly filled the width, the scale
 *  was pinned at 1 by the width and a third of the height went empty.
 *
 *  Once the count is settled and the scale is held by the height, whatever width is still spare
 *  goes into the gutters — into air, not into bigger cards, because a card's size is decided by
 *  what it holds and must not drift with the window. */
function place(cs, fw, fh) {
  let best = null;
  for (const C of [3, 5, 7]) {
    // Odd only: with an even count the centre line falls between two lanes and the hub has none.
    const plan = assign(cs, C);
    put(plan, GUT);
    let top = 0;
    let bottom = 0;
    for (const c of cs) {
      top = Math.min(top, c.cy - c.h / 2);
      bottom = Math.max(bottom, c.cy + c.h / 2);
    }
    const ch = bottom - top + PAD * 2;
    const scale = Math.min(fw / (C * W + (C - 1) * GUT + PAD * 2), fh / ch);
    if (!best || scale > best.scale) best = { scale, plan };
  }
  const { plan, scale } = best;
  const want = (fw / scale - PAD * 2 - plan.C * W) / Math.max(plan.C - 1, 1);
  put(plan, Math.min(Math.max(GUT, want), 250));
  return cs;
}

/** Which edge of a cluster the wire lands on — **and the label moves to that edge**, so the line
 *  always arrives at the name rather than at the corner furthest from it.
 *
 *  The edge follows from which lane the cluster is in, not from the shape of its box. Reading it
 *  off the box's proportions is what sent a short wide cluster in the left lane a wire "through
 *  the top", which then crossed the card above it and came out the other side. The wire always
 *  arrives from the hub's direction: off the hub's lane that is sideways, on it that is vertical.
 *
 *  The one exception is a cluster that is not first in its stack: its own sibling stands between
 *  it and the hub, so a vertical arrival is guaranteed to be hidden. Those come in from the side,
 *  which is what sends their wire around the sibling through the gutter beside the lane. */
function anchorOf(c) {
  if (!c.hubLane) return c.cx < 0 ? "right" : "left";
  if (!c.slot) return c.cy < 0 ? "bottom" : "top";
  return c.slot % 2 ? "right" : "left";
}

/** One branch, as a path. **A wire travels in a gutter**, for the reason `c.gx` is computed at
 *  all: the packing keeps the lanes from overlapping each other and says nothing about what
 *  stands between a wire's two ends.
 *
 *  It used to leave the hub radially — straight at the cluster it was going to — and the
 *  straight line from the hub to a label two ranks up crosses whatever lane is in between by
 *  construction, whatever curve is then fitted to it. Measured on the store this page was
 *  designed against: the wire to `songguo` spent 34% of its length inside the `also on its mind`
 *  cluster, 36px deep, and the one to `hi-agent` clipped the same chips from the other side.
 *
 *  So a side-entering wire leaves the hub level, turns up or down its gutter, and comes in level
 *  with the label — three straight intentions in one cubic, and no crossings at all on that
 *  store. One straight above or below the hub still runs straight, having nothing to go around.
 */
function route(c, hub, ex, ey) {
  const a = anchorOf(c);
  if (a === "top" || a === "bottom") {
    const dir = a === "top" ? 1 : -1;
    const sy = hub.y + dir * HUB_R;
    const d = Math.abs(ey - sy) * 0.5;
    return `M ${hub.x} ${sy} C ${hub.x} ${sy + dir * d}, ${ex} ${ey - dir * d}, ${ex} ${ey}`;
  }
  const gx = hub.x + c.gx;
  const sx = hub.x + Math.sign(c.gx) * HUB_R;
  return `M ${sx} ${hub.y} C ${gx} ${hub.y}, ${gx} ${ey}, ${ex} ${ey}`;
}

// ── the surface ───────────────────────────────────────────────────────────────

export default function Home() {
  const [tasks, setTasks] = useState(null);
  const [workers, setWorkers] = useState([]);
  const [minds, setMinds] = useState([]);
  const [views, setViews] = useState(() => new Map());
  const frame = useRef(null);
  const [box, setBox] = useState({ w: 0, h: 0 });
  const { openRef } = useViews();

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

  // **The chart is something you watch happen, and so are these.** Both used to read on
  // attention alone — the minds row because it cost one request per project, the view index
  // because it is a whole listing — and on the one surface built to *stay up*, "on attention"
  // means "once". The failure was worse than a stale number, because the ages on those chips
  // are recomputed from `Date.now()` every render: the labels kept ticking over a set frozen
  // at mount, so nothing on the screen said it had stopped reading. The minds row now costs
  // one request (`/api/facets` carries `modified`), and the view index is the same listing
  // the bookmarks band already re-reads every few seconds.
  //
  // **What that read costs on the far side, since this page holds it open all day.** Reading
  // the inventory queues up to three first pictures, which is a headless render each. The
  // queue is `system || bookmarked || in the trail` and never the whole tree, and a picture
  // that lands stops being wanted — on the live store all ten system views already had one
  // and nothing was bookmarked, so the steady state is an empty queue. What is *not* bounded
  // is a warm that keeps failing: `wants_shot` stays true, so a view that will never render
  // is retried for as long as this page is up. That was always true of the band and is now
  // true for longer.
  const loadMinds = useCallback(async () => {
    const index = await api.facets().catch(() => null);
    if (!index) return;
    const dimension = (index.dimensions || []).find((d) => d.dimension === "projects");
    const seen = (dimension?.subjects || [])
      .slice(0, MINDS_READ)
      .map((s) => ({ subject: s.subject, at: stamp(s.modified) }));
    seen.sort((a, b) => (b.at?.getTime() || 0) - (a.at?.getTime() || 0));
    setMinds(seen);
  }, []);

  // `shot_url` goes straight into an `<img src>`, and it is safe to: the core resolves it
  // against the base path the request arrived on before it hands it over (`list_views`,
  // `foundation::surfaces::reroot_path`). That is deliberately the producer's job and not
  // this file's — an `<img src>` is not carried by the `fetch` seam that rebases everything
  // else, and asking each reader to remember is how this surface shipped a broken picture on
  // every phone while the desktop looked fine. A path this view builds *itself* still needs
  // `url()`; there are none here.
  const loadViews = useCallback(async () => {
    const list = await api.views().catch(() => null);
    if (!Array.isArray(list)) return;
    const known = new Map();
    for (const v of list) {
      if (!v?.view_ref) continue;
      known.set(v.view_ref, { ref: v.view_ref, label: v.label, shot: v.shot_url || null });
    }
    setViews(known);
  }, []);

  useLive(load, { period: TEMPO.watching });
  useLive(loadMinds, { period: TEMPO.ledger });
  useLive(loadViews, { period: TEMPO.ledger });

  // The canvas is drawn to the frame it is handed, and the frame is the window — both ways.
  // Measured rather than assumed: this view is mounted at four widths that matter (a maximised
  // Mac, a half-width column, the headless renderer, a phone), a media query cannot hand
  // JavaScript a number to lay out with, and the height is half of what decides the scale.
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
  const trunks = tasks === null ? [] : model(tasks, workers, minds, views);
  // The three rungs were three pills in the corner once, reading `mouth · brain · memory`.
  // They were this architecture's words for itself, parked where nothing explained them, and
  // they answered a question nobody asks. One of the states they carried is worth a person's
  // attention, and only when it is true — with no Reaction registered the agent cannot hear
  // you, however busy the canvas looks.
  const deaf = tasks !== null && !workers.some((w) => w.role === "reaction");

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

  const wide = box.w >= 960 && box.h >= 520;
  const clusters = trunks.length > 0 ? trunks.map(cluster) : [];
  if (wide && clusters.length > 0) place(clusters, box.w, box.h);

  return (
    <div className="hi-home" ref={frame}>
      <style>{CSS}</style>
      {wide && clusters.length > 0 ? (
        <Canvas clusters={clusters} box={box} root={root} onOpen={openRef} />
      ) : (
        <>
          {root}
          {trunks.length === 0 ? (
            tasks !== null && <p className="hi-home__empty">{L.nothingOpen}</p>
          ) : (
            <Flow trunks={trunks} onOpen={openRef} />
          )}
        </>
      )}
    </div>
  );
}

// ── the canvas, drawn ─────────────────────────────────────────────────────────

function Canvas({ clusters, box, root, onOpen }) {
  // The canvas is whatever the content grew to; the frame then scales it to fit. That order is
  // the whole idea — sizing the layout to the frame first is what produced a sparse ring with a
  // hole in the middle, twice.
  const x0 = Math.min(...clusters.map((c) => c.cx - c.w / 2)) - PAD;
  const x1 = Math.max(...clusters.map((c) => c.cx + c.w / 2)) + PAD;
  const y0 = Math.min(...clusters.map((c) => c.cy - c.h / 2)) - PAD;
  const y1 = Math.max(...clusters.map((c) => c.cy + c.h / 2)) + PAD;
  const cw = x1 - x0;
  const ch = y1 - y0;
  const scale = Math.min(box.w / cw, box.h / ch, 1.6);
  const hub = { x: -x0, y: -y0 };

  const edge = (c) => {
    const left = c.cx - c.w / 2 - x0;
    const top = c.cy - c.h / 2 - y0;
    const a = anchorOf(c);
    if (a === "left") return [left, top + LABEL_H / 2];
    if (a === "right") return [left + c.w, top + LABEL_H / 2];
    if (a === "bottom") return [left + c.w / 2, top + c.h];
    return [left + c.w / 2, top];
  };

  return (
    <div className="hi-home__canvas" style={{ width: cw, height: ch, transform: `scale(${scale})` }}>
      <svg className="hi-home__wires" width={cw} height={ch} aria-hidden>
        {clusters.map((c) => (
          <path key={c.key} className="hi-home__wire" style={{ "--tone": TONE[c.tone] }}
            d={route(c, hub, ...edge(c))} />
        ))}
      </svg>
      <div className="hi-home__rootnode" style={{ left: hub.x, top: hub.y }}>{root}</div>
      {clusters.map((c) => {
        const a = anchorOf(c);
        const head = (
          <h2 className="hi-home__trunk">
            <span className="hi-home__trunk-name">{c.label}</span>
            {c.count !== null && c.count > 1 && <span className="hi-home__trunk-n">{c.count}</span>}
          </h2>
        );
        return (
          <section key={c.key} className="hi-home__cluster" data-anchor={a}
            style={{
              left: c.cx - c.w / 2 - x0,
              top: c.cy - c.h / 2 - y0,
              width: c.w,
              "--tone": TONE[c.tone],
            }}>
            {a !== "bottom" && head}
            {c.blocks.map((b) =>
              b.t === "sub" ? (
                <h3 key={b.id} className="hi-home__sub">
                  <span>{b.label}</span>
                  {b.count > 1 && <i>{b.count}</i>}
                </h3>
              ) : (
                <Card key={b.id} block={b} onOpen={onOpen} />
              ),
            )}
            {c.chips.length > 0 && (
              <div className="hi-home__chips">
                {c.chips.map((chip) => (
                  <Chip key={chip.id} chip={chip} onOpen={onOpen} />
                ))}
              </div>
            )}
            {a === "bottom" && head}
          </section>
        );
      })}
    </div>
  );
}

/** Who is on the row, as a mark on the row rather than a rank of its own. Filled and breathing
 *  while a session is running, hollow while one is parked, absent when nobody is on it — and a
 *  numeral only when there is more than one, because "1" beside a dot is the dot said twice. */
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
 *  naming it tells a reader nothing; a builder or a reviewer being on the row is worth the word. */
function kindOf(sessions) {
  const live = (sessions || []).find((worker) => worker.state === "running");
  const kind = live && (live.type || live.role);
  return kind && kind !== "general" ? L.doing[kind] || null : null;
}

/** **A card is a control when the row names a view**, which is the case for more than half of
 *  the open ledger: the ref is already written in the record and `POST /api/views/open` already
 *  takes it. Nothing had to be added to the wire for this — the file used to say a leaf could not
 *  open anything, and what that was really about was deep-linking `factory/tasks` at a given row,
 *  which is a different and still-missing thing. */
function Card({ block, onOpen }) {
  const { leaf, k } = block;
  const view = leaf.view;
  const kind = kindOf(leaf.sessions);
  const body = (
    <>
      {k === "P" && (
        <div className="hi-home__fig">
          <img src={view.shot} alt="" loading="lazy" />
        </div>
      )}
      <div className="hi-home__hd">
        <span className="hi-home__title">
          {k === "R" && view?.shot && <img className="hi-home__tiny" src={view.shot} alt="" loading="lazy" />}
          <span>{leaf.title}</span>
          {(leaf.marks || []).map((mark) => (
            <em key={mark} className="hi-home__mark">{mark}</em>
          ))}
          <OnIt sessions={leaf.sessions} />
        </span>
        {k === "T" && leaf.fact && <p className="hi-home__ask">{leaf.fact}</p>}
        <p className="hi-home__ln">
          <b className="hi-home__st">{L.state[leaf.tone]}</b>
          {/* **A finished card says what landed; an open one says how to get there.** On a row
              still in flight the picture is a door and the line is the sign on it, because
              nothing has happened yet that a sentence could report. On a finished row the
              picture is the thing itself, sitting right above the line, so spending that line
              on `open ‹label›` describes the click a reader is already looking at and drops
              the one fact only the record has. */}
          <span>
            {k === "P"
              ? leaf.tone === "done"
                ? leaf.fact || leaf.age
                : `${L.open} ${view.label || view.ref}`
              : k === "T"
                ? leaf.age
                : leaf.fact}
          </span>
          {kind && k === "R" && <em className="hi-home__kind">{kind}</em>}
        </p>
      </div>
    </>
  );
  if (!view) return <article className="hi-home__bx" data-k={k} style={{ height: block.h }}>{body}</article>;
  return (
    <article className="hi-home__bx" data-k={k} data-open="true" style={{ height: block.h }}>
      <button type="button" onClick={() => onOpen(view.ref)} title={view.ref}>{body}</button>
    </article>
  );
}

function Chip({ chip, onOpen }) {
  const inner = (
    <>
      {chip.view?.shot && <img className="hi-home__tiny" src={chip.view.shot} alt="" loading="lazy" />}
      <span className="hi-home__chip-t">{chip.title}</span>
      {chip.fact && <i>{chip.fact}</i>}
    </>
  );
  if (!chip.view) return <span className="hi-home__chip">{inner}</span>;
  return (
    <button type="button" className="hi-home__chip" data-open="true"
      onClick={() => onOpen(chip.view.ref)} title={chip.view.ref}>{inner}</button>
  );
}

// ── the narrow one ────────────────────────────────────────────────────────────
// A canvas needs width per lane and a phone has one lane's worth. Same model, same shapes, no
// geometry — the topics become sections down the page.

function Flow({ trunks, onOpen, rank = 1 }) {
  return (
    <div className="hi-home__flow" data-rank={rank}>
      {trunks.map((trunk) => {
        const c = cluster(trunk);
        return (
          <section key={trunk.key} style={{ "--tone": TONE[trunk.tone] }}>
            <h2 className="hi-home__trunk">
              <span className="hi-home__trunk-name">{trunk.label}</span>
              {c.count !== null && c.count > 1 && <span className="hi-home__trunk-n">{c.count}</span>}
            </h2>
            {c.blocks.map((b) =>
              b.t === "sub" ? (
                <h3 key={b.id} className="hi-home__sub"><span>{b.label}</span></h3>
              ) : (
                <Card key={b.id} block={{ ...b, h: undefined }} onOpen={onOpen} />
              ),
            )}
            {c.chips.length > 0 && (
              <div className="hi-home__chips">
                {c.chips.map((chip) => <Chip key={chip.id} chip={chip} onOpen={onOpen} />)}
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
}

// The only stylesheet in the file. A style prop cannot express a breakpoint, a pseudo-class
// or a descendant rule, and this surface wants all three; everything a prop *can* say that
// the geometry decides — a left, a top, a width — is still a prop, because it comes out of
// place() and belongs to the node it positions.
//
// NOTE: no backticks anywhere below, not even inside a comment. One ends the template early
// and the file stops being JavaScript — see no_bundled_view_has_a_backtick_inside_its_css.
const CSS = `
.hi-home {
  position: relative;
  box-sizing: border-box;
  min-height: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
  padding: max(24px, var(--hi-safe-top)) clamp(18px, 3vw, 52px) 118px;
  color: var(--fg);
  font-family: var(--font-display);
  /* **Three sizes and two weights, for the whole surface** (the sizes are TYPE, above, because
     the chip ruler measures in them). It had seven sizes and four weights, which is not a
     hierarchy — it is a hierarchy per element, and a wall of cards built that way reads as
     untidy however carefully each card was reasoned about. The three are the three things a
     card does: name something, say one thing about it, and carry a small mark. Every rank is
     told apart by size and colour; weight only ever separates a name from prose. */
  --t-lg: ${TYPE.lg}px;
  --t-md: ${TYPE.md}px;
  --t-sm: ${TYPE.sm}px;
}
.hi-home:has(.hi-home__canvas) { align-items: center; justify-content: center; overflow: hidden; }

/* ── the hub ── */
/* It carries nothing but absences, on purpose: whatever went into the largest type on the
   surface was already drawn on the branches. Two things still speak here and both are things
   no branch can show — whether anything at all is moving, and the one failure that makes the
   rest of the canvas a lie. */
.hi-home__rootnode { position: absolute; transform: translate(-50%, -50%); z-index: 2; }
.hi-home__root { display: flex; flex-direction: column; align-items: center; gap: 7px; }
.hi-home__pip {
  flex: 0 0 auto;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: var(--fg-mute);
  box-shadow: 0 0 0 7px color-mix(in srgb, var(--fg-mute) 9%, transparent);
}
.hi-home__pip[data-live] {
  background: var(--accent);
  animation: hi-home-pulse 2.6s var(--ease) infinite;
}
@keyframes hi-home-pulse {
  0%, 100% { box-shadow: 0 0 0 7px color-mix(in srgb, var(--accent) 12%, transparent); }
  50% { box-shadow: 0 0 0 14px color-mix(in srgb, var(--accent) 3%, transparent); }
}
.hi-home__meta { white-space: nowrap; font-size: var(--t-md); font-weight: 700; color: var(--fg-mute); }
.hi-home__deaf { color: var(--danger); font-weight: 700; }
.hi-home__empty { font-size: var(--t-lg); color: var(--fg-mute); margin: 26px 0 0; }

/* ── the canvas ── */
.hi-home__canvas { position: relative; flex: 0 0 auto; transform-origin: center center; }
.hi-home__wires { position: absolute; inset: 0; overflow: visible; pointer-events: none; }
.hi-home__wire {
  fill: none;
  stroke: var(--tone);
  stroke-width: 1.5;
  stroke-linecap: round;
  opacity: .42;
}

.hi-home__cluster { position: absolute; box-sizing: border-box; display: flex;
  flex-direction: column; align-items: stretch; gap: 9px; }
/* **A heading is as wide as its words and hangs off the edge the wire arrives at.** It used to
   be a stretched row with a paper ground behind the name alone, to keep a wire that passed
   behind it from reading as a strikethrough through the word. The ground is gone with the reason
   for it: wires travel in the gutters now and stop at the edge of the label rather than running
   through it. It was never free — the paper token is a *gradient*, so painting it in a 30px
   box starts it over and the label reads as a lighter chip laid on the page: plainly visible
   in the dark skin, and invisible in the light one only because both its stops are white. Both
   ranks take the same treatment; the count rides on the heading rather than outside it. */
.hi-home__trunk, .hi-home__sub {
  margin: 0;
  align-self: flex-start;
  width: fit-content;
  max-width: 100%;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
}
.hi-home__trunk { height: 30px; }
.hi-home__sub { height: 24px; }
.hi-home__cluster[data-anchor="right"] .hi-home__trunk,
.hi-home__cluster[data-anchor="right"] .hi-home__sub { flex-direction: row-reverse; align-self: flex-end; }
.hi-home__cluster[data-anchor="top"] .hi-home__trunk,
.hi-home__cluster[data-anchor="top"] .hi-home__sub,
.hi-home__cluster[data-anchor="bottom"] .hi-home__trunk,
.hi-home__cluster[data-anchor="bottom"] .hi-home__sub { align-self: center; }
.hi-home__trunk-name, .hi-home__sub > span {
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0;
}
.hi-home__trunk-name { font-size: var(--t-lg); font-weight: 700; letter-spacing: -.012em; color: var(--tone); }
/* A rank below a topic is a part of it, so it is quieter than the thing it is part of. */
.hi-home__sub { font-size: var(--t-md); font-weight: 700; color: var(--fg-dim); }
/* **Every small mark is the same mark**: a topic's count, a sub-topic's, how many sessions are
   on a row, how long ago a thing was thought about. They were 11px, 10.5px and 10px, three
   sizes for one idea and nothing on the screen saying why. */
.hi-home__trunk-n, .hi-home__sub i, .hi-home__on i, .hi-home__chip i {
  flex: 0 0 auto;
  font-style: normal;
  font-family: var(--font-mono);
  font-size: var(--t-sm);
  font-weight: 700;
  color: var(--fg-mute);
}
.hi-home__trunk-n { color: var(--tone); opacity: .55; }

.hi-home__bx {
  box-sizing: border-box;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--surface-border);
  border-radius: 10px;
  background: var(--surface);
}
.hi-home__bx[data-k="P"] { padding: 0 0 10px; }
.hi-home__bx[data-k="T"] { padding: 11px 13px; }
.hi-home__bx[data-k="R"] { padding: 9px 12px; justify-content: center; }
/* The whole card is the control when the row names a view. A button, so it is reachable from a
   keyboard and announced as one; everything else about it is the card. */
.hi-home__bx[data-open] > button {
  all: unset;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  cursor: pointer;
}
.hi-home__bx[data-open]:hover { border-color: color-mix(in srgb, var(--accent) 55%, var(--surface-border)); }
.hi-home__bx[data-open]:has(:focus-visible) { outline: 2px solid var(--accent); outline-offset: 2px; }

/* The picture is never cropped, and the shapes are not uniform — the same view re-shot in a
   narrow window comes back portrait. The card is a fixed size, so a tall picture must leave room
   at its sides; making that room a mat, and giving the picture its own edge and shadow, is what
   makes it read as a page rather than as a failed render. */
.hi-home__fig {
  height: 168px;
  flex: 0 0 auto;
  background: color-mix(in srgb, var(--fg) 6%, var(--paper));
  border-bottom: 1px solid var(--surface-border);
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  padding: 8px;
}
.hi-home__fig img {
  display: block;
  max-width: 100%;
  max-height: 100%;
  width: auto;
  height: auto;
  border-radius: 4px;
  box-shadow: 0 1px 4px color-mix(in srgb, var(--fg) 18%, transparent);
}
.hi-home__hd { min-width: 0; display: flex; flex-direction: column; gap: 3px; }
.hi-home__bx[data-k="P"] .hi-home__hd { padding: 9px 12px 0; }
.hi-home__bx[data-k="T"] .hi-home__hd { flex: 1; }
.hi-home__title {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
  font-size: var(--t-lg);
  font-weight: 700;
  letter-spacing: -.012em;
  line-height: 1.3;
}
.hi-home__title > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
/* **One thumbnail**, the same size with the same edge in a card's title as in a chip. Two
   sizes of the same thing is half of what makes a wall of them read as untidy; the other half
   was one of them having an edge and the other not. */
.hi-home__tiny {
  flex: 0 0 auto;
  width: 26px;
  height: 16px;
  object-fit: cover;
  object-position: top left;
  border-radius: 4px;
  border: 1px solid var(--surface-border);
}
/* A text card has no picture, so its body fills the space one would have taken. */
.hi-home__ask {
  margin: 0;
  flex: 1;
  font-size: var(--t-md);
  line-height: 1.5;
  color: var(--fg-dim);
  display: -webkit-box;
  -webkit-line-clamp: 4;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.hi-home__ln {
  margin: 0;
  font-size: var(--t-md);
  color: var(--fg-mute);
  display: flex;
  align-items: baseline;
  gap: 7px;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
}
/* The sentence is its own element so it can ellipsise. Bare text in a flex row is an anonymous
   item and text-overflow has nothing to apply to, so an overrun was cut mid-word at the card's
   edge — a fact that ended in "· just no" read as a card that had failed rather than a line
   that was long. */
.hi-home__ln > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
/* The one place a state is named, in the one colour it is worth spending. */
.hi-home__st { flex: 0 0 auto; font-size: var(--t-md); font-weight: 700; color: var(--tone); }
.hi-home__kind { flex: 0 0 auto; font-style: normal; font-weight: 700; color: var(--accent); }
/* A topic this row is under that did not earn a rank of its own. */
.hi-home__mark {
  flex: 0 0 auto;
  font-style: normal;
  font-size: var(--t-sm);
  font-weight: 700;
  color: var(--fg-mute);
  border: 1px solid var(--surface-border);
  background: var(--paper);
  border-radius: 4px;
  padding: 1px 6px;
}
.hi-home__on { flex: 0 0 auto; display: flex; align-items: center; gap: 4px; }
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

/* A cluster is one column of fixed width, so its chips have to live inside it: one chip whose
   title runs longer than the column would otherwise push the whole cluster out past its lane. */
.hi-home__chips { display: flex; gap: 6px; flex-wrap: wrap; width: 100%; min-width: 0; }
.hi-home__cluster[data-anchor="right"] .hi-home__chips { justify-content: flex-end; }
.hi-home__cluster[data-anchor="top"] .hi-home__chips,
.hi-home__cluster[data-anchor="bottom"] .hi-home__chips { justify-content: center; }
.hi-home__chip {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  box-sizing: border-box;
  height: 24px;
  padding: 0 9px;
  border-radius: 999px;
  border: 1px solid var(--surface-border);
  background: var(--surface);
  font-size: var(--t-md);
  font-weight: 700;
  color: var(--fg-dim);
  white-space: nowrap;
  font-family: inherit;
  max-width: 100%;
  min-width: 0;
}
.hi-home__chip-t { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
button.hi-home__chip { cursor: pointer; }
button.hi-home__chip:hover { border-color: color-mix(in srgb, var(--accent) 55%, var(--surface-border)); }
button.hi-home__chip:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }

/* ── the narrow one ── */
.hi-home__flow { margin-top: 22px; display: flex; flex-direction: column; gap: 26px; }
.hi-home__flow section { display: flex; flex-direction: column; gap: 9px; }
.hi-home__flow .hi-home__bx { height: auto; }
.hi-home__flow .hi-home__fig { height: 150px; }
`;
