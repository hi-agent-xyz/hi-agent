import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

// The factory compiler transforms one module and the host resolves its bare imports.
// Evaluate the pure projection section with the REAL layout engine, without a browser.
const require = createRequire(new URL("../../../appearance/web/package.json", import.meta.url));
const { flextree } = require("d3-flextree");
const source = readFileSync(new URL("./home.jsx", import.meta.url), "utf8");
const pure = source.slice(0, source.indexOf("export default function Home")).replace(/^import .*;$/gm, "");
const { buildHome, arrange, stage, zoomAround, clampZoom, TONE, stateOf, hands, childIndex, normalizeSession, emphasis, groupIcon, branchHues, branchTones, branchPaint, focusOn, trail, anchorAt, anchorPoint } = runInNewContext(
  `${pure}\n;({ buildHome, arrange, stage, zoomAround, clampZoom, TONE, stateOf, hands, childIndex, normalizeSession, emphasis, groupIcon, branchHues, branchTones, branchPaint, focusOn, trail, anchorAt, anchorPoint });`,
  { flextree, document: { documentElement: { lang: "en" } }, navigator: { language: "en" } },
);
const NOW = Date.parse("2026-09-11T12:00:00Z");
const hoursAgo = (h) => new Date(NOW - h * 3600000).toISOString();
const task = (subject, status = "doing", hours = 1) => ({ subject, title: subject, status, statusSince: hoursAgo(hours), refs: [], extra: [], files: [] });
const tagged = (subject, names, status = "doing") => ({ ...task(subject, status), extra: [{ key: "systems", value: names }] });
const worker = (id, role = "worker", extra = {}) => ({ run: "0123456789ab", id, role, state: "running", title: `Work ${id}`, started: hoursAgo(2), state_since: hoursAgo(1), ...extra });
const project = (input) => buildHome(input, NOW);
const ofKind = (model, kind) => model.nodes.filter((n) => n.kind === kind);
// The projection runs in its own realm, so arrays come back cross-realm and deepEqual
// refuses them. Every comparison below is made against a copy built on this side.
const list = (values) => [...values];
const titles = (model, kind) => list(ofKind(model, kind).map((n) => n.title)).sort();

function assertConnected(model) {
  const ids = new Set(model.nodes.map((n) => n.id));
  assert.equal(ids.size, model.nodes.length, "unique identities");
  const parent = new Map();
  for (const edge of model.edges) {
    assert.ok(ids.has(edge.from) && ids.has(edge.to), "no dangling edge");
    if (!edge.primary) continue;
    assert.ok(!parent.has(edge.to), `exactly one primary parent (${edge.to})`); parent.set(edge.to, edge.from);
  }
  for (const node of model.nodes) {
    let id = node.id;
    const seen = new Set();
    while (id !== "core") {
      assert.ok(!seen.has(id), "acyclic"); seen.add(id);
      assert.ok(parent.has(id), `${id} must reach the core`); id = parent.get(id);
    }
  }
}

test("what is open is in hand however old it is; what is closed stays only while it is recent", () => {
  const model = project({ tasks: [
    task("stale-but-open", "todo", 900), task("serving", "serving", 900),
    task("just-closed", "done", 3), task("closed-yesterday", "done", 30),
  ] });
  assert.deepEqual(titles(model, "task"), ["just-closed", "serving", "stale-but-open"]);
  assertConnected(model);
});

test("the boundary is inclusive and uses closure time, not creation or last rewrite", () => {
  const at = (h) => project({ tasks: [{ ...task("a", "done", 0), completedAt: hoursAgo(h), statusSince: hoursAgo(0) }] });
  assert.equal(ofKind(at(23.9), "task").length, 1);
  assert.equal(ofKind(at(25), "task").length, 0);
});

test("an unknown closure time reads as old, not as timeless", () => {
  // The inversion this surface was rebuilt for. A record with no closure stamp used to be
  // retained forever on the rule that a missing time must not be fabricated; on a real
  // instance that was nine permanent residents whose records were over a month stale.
  const model = project({ tasks: [{ ...task("a", "done"), completedAt: null, statusSince: null }] });
  assert.equal(ofKind(model, "task").length, 0);
  // Still never fabricated for something that IS shown: an open task with no stamp stays.
  const open = project({ tasks: [{ ...task("b", "doing"), statusSince: null }] });
  assert.equal(ofKind(open, "task").length, 1);
  assert.equal(open.nodes.find((n) => n.kind === "task").data.endedAt, null);
});

test("a live session does not re-admit its own expired task", () => {
  // The single biggest leak in the surface this replaced: any recent session naming a task
  // kept that task drawn as a peer of the work in hand, however long ago it had closed.
  const model = project({ tasks: [task("old", "done", 900)], workers: [worker("w", "worker", { subject: "old" })] });
  assert.equal(ofKind(model, "task").length, 0);
  const activity = ofKind(model, "activity")[0];
  assert.equal(activity.title, "Work w");
  assert.equal(activity.data.taskId, null);
  assertConnected(model);
});

test("an ended session is not this surface's subject at all", () => {
  const model = project({ workers: [worker("live")], ended: [{ run: "0123456789ab", session: "gone", ended: hoursAgo(1) }] });
  assert.deepEqual(titles(model, "activity"), ["Work live"]);
});

test("conversation and coordination make the core; a session on a drawn task is a line on it", () => {
  const model = project({ tasks: [task("a")], workers: [worker("r", "reaction"), worker("c", "cognition"),
    worker("f", "reflection"), worker("w1", "worker", { subject: "a" }), worker("w2", "worker", { subject: "a" })] });
  assert.deepEqual(list(model.nodes[0].data.sessions.map((s) => s.role)).sort(), ["cognition", "reaction"]);
  // Reflection is the only card: the two working the task are that task card's own state.
  assert.deepEqual(titles(model, "activity"), ["Review"]);
  const drawn = ofKind(model, "task")[0];
  assert.deepEqual(list(drawn.data.sessions.map((s) => s.slug)).sort(), ["w1", "w2"]);
  assert.deepEqual(list(childIndex(model).get("task:a")), [], "and it hangs nothing below the card");
  assert.ok(drawn.sourceRefs.some((r) => r.kind === "session"), "the sessions stay source references");
  assertConnected(model);
});

test("one word for a task, and a mark for each hand on it", () => {
  // The grid the two-line card promised mostly does not exist: nothing is being worked on while
  // it is still to do, and a closed row is closed whatever is still warm beside it.
  const of = (status, workers) => ofKind(project({ tasks: [task("a", status)], workers }), "task")[0];
  const on = (state, extra = {}) => worker(`w${state}`, "worker", { subject: "a", state, ...extra });
  assert.equal(stateOf(of("todo", [])), "todo");
  assert.equal(stateOf(of("doing", [on("running")])), "doing", "the word stays the ledger's");
  assert.equal(stateOf(of("serving", [on("idle")])), "serving");
  assert.equal(stateOf(of("done", [on("idle")])), "done");
  // The one thing a session says that the ledger cannot: its last turn was cut off.
  const cut = of("doing", [on("idle", { last_turn: { outcome: "failed" } })]);
  assert.equal(stateOf(cut), "failed");
  assert.equal(TONE[stateOf(cut)], "var(--danger)");
  // Not while something else on the same row is still running — that row is progressing.
  assert.equal(stateOf(of("doing", [on("idle", { last_turn: { outcome: "failed" } }), on("running")])), "doing");
  // A closed row is closed: a stale failure under it is not the card's word.
  assert.equal(stateOf(of("cancelled", [on("idle", { last_turn: { outcome: "interrupted" } })])), "cancelled");
  // Running first, then the newest; three marks at most and the rest is a number.
  const many = of("doing", [on("idle"), on("running"), worker("w3", "worker", { subject: "a", state: "waiting" }),
    worker("w4", "worker", { subject: "a", state: "idle" })]);
  const { shown, more, title } = hands(many);
  assert.equal(shown[0].state, "running");
  assert.equal(shown.length, 3);
  assert.equal(more, 1);
  assert.match(title, /Work wrunning · Working/, "every hand is named in the hover text");
});

test("the agent's own upkeep is a group code draws: Reflection and every session with no subject", () => {
  // Dispatch refuses a subject to the kinds that serve no one task and to anything Reflection
  // starts, so no subject means upkeep by construction — nothing here reads a title or a kind.
  const model = project({ tasks: [task("a")], groups: [{ label: "Work", members: ["a"] }],
    workers: [worker("reflection", "reflection"), worker("sweep", "worker", { type: "task-manager", owner: "cognition" }),
      worker("reader", "worker", { owner: "reflection" }), worker("builder", "worker", { subject: "a" }),
      worker("orphan", "worker", { subject: "aged-out" })] });
  const kids = childIndex(model);
  const upkeep = model.nodes.find((n) => n.id === "upkeep");
  assert.equal(upkeep.kind, "group");
  assert.equal(upkeep.title, "Upkeep");
  assert.deepEqual(list(ofKind(model, "task")[0].data.sessions.map((s) => s.slug)), ["builder"], "a session on a drawn task is not in here");
  assert.deepEqual(list(kids.get("upkeep").map((n) => n.id)).sort(),
    ["session:0123456789ab:reader", "session:0123456789ab:reflection", "session:0123456789ab:sweep"]);
  assert.equal(kids.get("upkeep").find((n) => n.data.session.role === "reflection").title, "Review");
  // One whose task is not drawn is still somebody's work, and stays a card on the core.
  assert.ok(kids.get("core").some((n) => n.id === "session:0123456789ab:orphan"));
  // It comes after the person's own groups, takes a hue of its own, and can be taken as the centre.
  assert.deepEqual(list(kids.get("core").filter((n) => n.kind === "group").map((n) => n.id)), ["group:Work", "upkeep"]);
  const tones = branchTones(model);
  assert.notEqual(tones.get("session:0123456789ab:sweep"), undefined);
  assert.notEqual(tones.get("upkeep"), tones.get("group:Work"));
  assert.equal(focusOn(model, "upkeep").nodes.length, 4);
  assert.deepEqual(list(focusOn(model, "group:Work").nodes.map((n) => n.id)), ["group:Work", "task:a"]);
  assertConnected(model);
  // Nothing live that is upkeep draws no heading.
  assert.equal(project({ workers: [worker("builder", "worker", { subject: "a" })], tasks: [task("a")] }).nodes.some((n) => n.id === "upkeep"), false);
});

test("subject decides which card a session lands on, never its technical owner", () => {
  const model = project({ tasks: [task("a"), task("b")],
    workers: [worker("boss", "worker", { subject: "a" }), worker("hand", "worker", { subject: "b", owner: "boss" })] });
  const on = (subject) => ofKind(model, "task").find((n) => n.id === `task:${subject}`).data.sessions;
  assert.deepEqual(list(on("b").map((s) => s.slug)), ["hand"], "the owner's task does not claim it");
  assert.equal(on("b")[0].ownerSessionId, "session:0123456789ab:boss", "the owner stays inspectable");
  assert.deepEqual(list(on("a").map((s) => s.slug)), ["boss"]);
});

test("a missing task or an owner cycle never disconnects a session", () => {
  const model = project({ workers: [worker("x", "worker", { subject: "nope", owner: "y" }), worker("y", "worker", { owner: "x" })] });
  assertConnected(model);
});

test("idle is live, last-turn failure is not session termination, stale doing is not current", () => {
  const idle = normalizeSession(worker("i", "worker", { state: "idle", doing: "old thing" }));
  assert.equal(idle.online, true); assert.equal(idle.currentAction, null);
  const failed = { kind: "activity", data: { session: normalizeSession(worker("f", "worker", { state: "idle", last_turn: { outcome: "failed" } })) } };
  assert.equal(stateOf(failed), "failed");
});

test("a task waiting on the person says so on its card, and a closed one never does", () => {
  const waiting = { kind: "waiting", at: hoursAgo(0.25), text: "needs a root password on the box" };
  const model = project({ tasks: [{ ...task("open"), latest: waiting }, { ...task("closed", "done", 0), latest: waiting },
    { ...task("moved-on"), latest: { kind: "update", at: hoursAgo(0.1), text: "went round it" } }] });
  const state = (subject) => stateOf(model.nodes.find((n) => n.id === `task:${subject}`));
  assert.equal(state("open"), "needsYou");
  assert.equal(state("closed"), "done");
  assert.equal(state("moved-on"), "doing");
  assert.ok(TONE.needsYou);

  // A cut turn says our side stopped; a wait says the next step is theirs, and that is the
  // one they can act on.
  const cut = project({ tasks: [{ ...task("open"), latest: waiting }],
    workers: [worker("w", "worker", { subject: "open", state: "idle", last_turn: { outcome: "failed" } })] });
  assert.equal(stateOf(cut.nodes.find((n) => n.id === "task:open")), "needsYou");
});

test("overview uses public messages and factual transitions, never worker tail or reasoning", () => {
  const model = project({ tasks: [task("shipped", "done", 2)], workers: [worker("w", "worker", { doing: "internal narration" })],
    messages: [{ id: "m1", role: "user", text: "where are we", ts: hoursAgo(1) },
      { id: "m2", role: "agent", text: "shipped it", ts: hoursAgo(0.5) },
      { id: "m3", role: "tool", text: "internal narration", ts: hoursAgo(0.4) }] });
  const overview = ofKind(model, "overview");
  assert.ok(overview.every((n) => !n.data.text.includes("internal narration")));
  assert.deepEqual(list(overview.map((n) => n.data.category)).sort(), ["context", "update", "update"]);
  // Closed-but-not-recent work reports nothing: there is no transition inside the window.
  const quiet = project({ tasks: [task("old", "done", 900)] });
  assert.equal(ofKind(quiet, "overview").length, 0);
});

test("what a task touches never groups it; only the arrangement does", () => {
  // `systems` names the records a task touches, not what it belongs to: a birthday deck
  // whose photos came over Feishu is not part of a Feishu project. With no arrangement
  // written, every task is its own branch however much its wording overlaps its neighbour's.
  const model = project({ tasks: [tagged("deck", "feishu, wecom"), tagged("brief", "feishu"), task("plain")] });
  assert.deepEqual(list(childIndex(model).get("core").map((n) => n.id)).sort(), ["task:brief", "task:deck", "task:plain"]);
  assert.deepEqual(list(new Set(model.nodes.map((n) => n.kind))).sort(), ["core", "task"]);
  assertConnected(model);
});

test("the arrangement puts a task under its group, and leaves the rest on the core", () => {
  const model = project({
    tasks: [task("kt8-046"), task("cantonese-table"), task("vocabulary-book")],
    groups: [{ label: "KTV", note: "9/16 说是一摊事", members: ["kt8-046", "cantonese-table"] }] });
  const children = childIndex(model);
  assert.deepEqual(list(children.get("core").map((n) => n.id)).sort(), ["group:KTV", "task:vocabulary-book"]);
  assert.deepEqual(list(children.get("group:KTV").map((n) => n.id)), ["task:kt8-046", "task:cantonese-table"]);
  // The note is the only thing a group says beyond its name, and it is carried, not dropped.
  assert.equal(model.nodes.find((n) => n.id === "group:KTV").data.note, "9/16 说是一摊事");
  assertConnected(model);
});

test("a group whose tasks are all gone is not a heading standing on its own", () => {
  // The record is not the ledger: tasks close and age off this surface while their line
  // still stands in the arrangement. An empty group is structure where content used to be.
  const model = project({
    tasks: [task("still-open")],
    groups: [{ label: "KTV", members: ["closed-last-month", "never-existed"] },
      { label: "在办", members: ["still-open"] }] });
  assert.deepEqual(list(model.nodes.filter((n) => n.kind === "group").map((n) => n.title)), ["在办"]);
  assertConnected(model);
});

test("groups are laid out in the order they were arranged in, ahead of what is ungrouped", () => {
  const model = project({
    tasks: [task("a-first-alphabetically"), task("m-in-a-late-group"), task("z-in-an-early-group")],
    groups: [{ label: "早", members: ["z-in-an-early-group"] }, { label: "晚", members: ["m-in-a-late-group"] }] });
  // Branches are fed to the layout in that order and then dealt to whichever side is
  // lighter, so the order survives *within* a side — which is the part this can assert, and
  // exactly as much as the person gets. Which side a branch lands on is `home.md` § Open.
  const onCore = new Set(model.edges.filter((e) => e.primary && e.from === "core").map((e) => e.to));
  const sides = [[], []];
  for (const row of arrange(model).placed.slice(1)) {
    if (onCore.has(row.node.id)) sides[row.dir > 0 ? 0 : 1].push(row.node);
  }
  for (const side of sides) {
    const titles = side.map((n) => n.title);
    assert.deepEqual(titles, list(side).sort((a, b) => (a.kind === "group" ? 0 : 1) - (b.kind === "group" ? 0 : 1)
      || (a.kind === "group" ? a.data.index - b.data.index : 0)).map((n) => n.title),
    `groups first, in the record's order, then what nobody placed: ${titles}`);
  }
  assert.deepEqual(list(sides.flat().filter((n) => n.kind === "group").map((n) => n.title)).sort(), ["早", "晚"]);
});

test("a live session follows its task into the group, and a task claimed twice stays in the first", () => {
  const model = project({
    tasks: [task("kt8-046")],
    workers: [worker("w1", "worker", { subject: "kt8-046" })],
    groups: [{ label: "KTV", members: ["kt8-046"] }, { label: "别的", members: ["kt8-046"] }] });
  const children = childIndex(model);
  assert.deepEqual(list(children.get("group:KTV").map((n) => n.id)), ["task:kt8-046"]);
  assert.equal(children.get("core").some((n) => n.id === "group:别的"), false, "the second claim is not a group");
  assert.deepEqual(list(ofKind(model, "task")[0].data.sessions.map((s) => s.slug)), ["w1"], "the session is a line on the card in the group");
  assert.deepEqual(list(children.get("task:kt8-046")), [], "and not a card of its own beside it");
  assertConnected(model);
});

test("a group inside a group draws core, group, inner group, card", () => {
  const model = project({
    tasks: [task("rollup"), task("site-survey"), task("field-notes"), task("vocabulary-book"), task("loose")],
    groups: [
      { label: "Client work", note: "one company", members: ["rollup"], groups: [
        { label: "Site", members: ["site-survey"] },
        { label: "Research", groups: [{ label: "Notes", members: ["field-notes"] }] },
      ] },
      { label: "Home", members: ["vocabulary-book"] },
    ] });
  const children = childIndex(model);
  assert.deepEqual(list(children.get("core").map((n) => n.id)).sort(), ["group:Client work", "group:Home", "task:loose"]);
  assert.deepEqual(list(children.get("group:Client work").map((n) => n.id)),
    ["task:rollup", "group:Site", "group:Research"], "its own task first, then its groups in the record's order");
  assert.deepEqual(list(children.get("group:Research").map((n) => n.id)), ["group:Notes"], "a group holding only a group is still drawn");
  assert.deepEqual(list(children.get("group:Notes").map((n) => n.id)), ["task:field-notes"]);
  assert.equal(ofKind(model, "group").length, 5, "each group once");
  assertConnected(model);
  const placed = arrange(model).placed;
  assert.equal(placed.length, model.nodes.filter((n) => n.kind !== "overview").length, "every node is laid out once");
});

test("inner groups keep the record's order, whichever of their tasks is drawn first", () => {
  const model = project({
    tasks: [task("late"), task("early")],
    groups: [{ label: "Work", groups: [{ label: "First", members: ["early"] }, { label: "Second", members: ["late"] }] }] });
  assert.deepEqual(list(childIndex(model).get("group:Work").map((n) => n.title)), ["First", "Second"]);
});

test("an inner group with nothing drawn, and the groups above it, are not headings", () => {
  const model = project({
    tasks: [task("a"), task("closed-long-ago", "done", 900)],
    groups: [{ label: "Kept", members: ["a"] },
      { label: "Outer", groups: [{ label: "Inner", members: ["closed-long-ago", "never-existed"] }] }] });
  assert.deepEqual(list(ofKind(model, "group").map((n) => n.title)), ["Kept"]);
  assertConnected(model);
});

test("a record naming one label twice keeps the first group, so nothing has two parents", () => {
  const model = project({
    tasks: [task("a"), task("b")],
    groups: [{ label: "Work", members: ["a"] }, { label: "Other", groups: [{ label: "Work", members: ["b"] }] }] });
  assert.deepEqual(list(childIndex(model).get("group:Work").map((n) => n.id)), ["task:a"]);
  assert.equal(ofKind(model, "group").length, 1, "the second Work and the Other it made are not drawn");
  assertConnected(model);
});

test("a group is a label, and a card is still a card beneath it", () => {
  const model = project({ tasks: [task("kt8-046")], groups: [{ label: "KTV", members: ["kt8-046"] }] });
  const placed = arrange(model).placed;
  const group = placed.find((p) => p.node.kind === "group");
  const card = placed.find((p) => p.node.kind === "task");
  assert.deepEqual([group.w, group.h], [144, 56], "a compact heading has room for two lines");
  assert.deepEqual([card.w, card.h], [240, 135]);
  assert.equal(card.x - (group.x + group.w), 48, "a shorter gutter preserves the connector");
});

test("a group wears the icon drawn for it, and the default until one is", () => {
  const model = project({ tasks: [task("kt8-046"), task("vocabulary-book")],
    groups: [{ label: "KTV", icon: "drive/home/icons/0192.png", members: ["kt8-046"] },
      { label: "学习类", members: ["vocabulary-book"] }] });
  const icons = Object.fromEntries(ofKind(model, "group").map((n) => [n.title, groupIcon(n.data.icon)]));
  assert.equal(icons.KTV, "/api/drive/file/home/icons/0192.png");
  assert.equal(icons["学习类"], "/api/home/group-icon");
  // Only a drive ref is fetched; anything else is not this surface's to guess at.
  assert.equal(groupIcon("https://example.com/icon.png"), "/api/home/group-icon");
  assert.equal(groupIcon("drive/home/icons/a b.png"), "/api/drive/file/home/icons/a%20b.png");
});

test("every group up to eight has a hue of its own, and a group appended later moves none", () => {
  const labels = ["监听项", "KNQ · 汇总", "KNQ · 赛马专家", "KNQ · 视觉·场地", "KNQ · 北控视频", "生活类", "学习类"];
  const hues = branchHues(labels);
  assert.equal(new Set(hues.values()).size, labels.length, "no two groups share a hue");
  const more = branchHues([...labels, "自己身上的毛病"]);
  assert.equal(new Set(more.values()).size, 8);
  for (const label of labels) assert.equal(more.get(label), hues.get(label), `${label} keeps its hue`);
  // Past eight there is nothing left to be distinct with, and a colour is reused rather than invented.
  assert.equal(new Set(branchHues([...labels, "8", "9"]).values()).size, 8);
});

test("a wire is its first-level group's one colour at every depth, and never the status", () => {
  const views = [{ view_ref: "views/one", label: "One", shot_url: "/one.png" }];
  const input = {
    tasks: [task("rollup", "serving"), task("horse", "todo"), { ...task("court", "doing"), refs: ["views/one"] },
      task("vocab"), task("loose")],
    views, workers: [worker("w1", "worker", { subject: "court" }), worker("w2", "worker", { subject: "court" })],
    groups: [{ label: "KNQ", members: ["rollup"], groups: [
      { label: "赛马专家", members: ["horse"] }, { label: "视觉·场地", members: ["court"] }] },
    { label: "学习类", members: ["vocab"] }] };
  const model = project(input);
  const tones = branchTones(model);
  const knq = tones.get("group:KNQ");
  assert.notEqual(knq, tones.get("group:学习类"), "two first-level groups, two colours");

  // Tasks, inner groups, their tasks and pictures: all of it is KNQ's colour.
  const under = model.nodes.map((n) => n.id).filter((id) => !["core", "group:KNQ", "group:学习类", "task:vocab", "task:loose"].includes(id));
  assert.ok(under.some((id) => id.startsWith("result:")) && under.some((id) => id.startsWith("group:")));
  for (const id of under) assert.equal(tones.get(id), knq, `${id} wears KNQ's colour`);
  for (const wire of arrange(model).wires.filter((w) => under.includes(w.to))) assert.equal(wire.paint, branchPaint(knq));

  // Nothing in no group has a category to show.
  assert.equal(tones.get("task:loose"), undefined);
  assert.equal(arrange(model).wires.find((w) => w.to === "task:loose").paint, "var(--work-line)");

  // Status changes nothing.
  const flipped = project({ ...input, tasks: input.tasks.map((t) => ({ ...t, status: t.status === "doing" ? "todo" : "doing" })) });
  const paints = (m) => list(arrange(m).wires.map((w) => `${w.to}=${w.paint}`)).sort();
  assert.deepEqual(paints(flipped), paints(model));
  assert.match(branchPaint(knq), /^oklch\(var\(--work-branch-l\) var\(--work-branch-c\) [0-9]+\)$/);
  assert.match(branchPaint(knq, "label"), /^oklch\(var\(--work-label-l\) var\(--work-branch-c\) [0-9]+\)$/);
});

test("a result with no picture is not on Home at all", () => {
  const model = project({
    tasks: [{ ...task("a"), refs: ["views/deck"], files: [{ path: "notes.md" }] }],
    views: [{ view_ref: "views/deck", label: "Deck" }, { view_ref: "views/other" }] });
  assert.equal(model.nodes.length, 2, "core and the task, and nothing else");
  assert.equal(ofKind(model, "task")[0].data.results.length, 0, "and nothing on the task counts it");
  assert.equal(arrange(model).placed.length, 2);
});

test("every picture is an image node below its task, and no card wears one", () => {
  const shots = project({
    tasks: [{ ...task("deck"), refs: ["views/notes", "views/slide", "views/other-slide"] }],
    views: [{ view_ref: "views/notes", label: "Notes" },
      { view_ref: "views/slide", label: "Slide", shot_url: "/a.png" },
      { view_ref: "views/other-slide", label: "Other", shot_url: "/b.png" }] });
  // One appearance per kind: a card is a card whether or not the task made a picture, and
  // every picture is a tile. Which one a result got used to depend on its place in `refs`.
  const drawn = arrange(shots).placed.find((p) => p.node.id === "task:deck");
  assert.deepEqual([drawn.w, drawn.h], [240, 135], "a picture never widens or heightens the card");
  // Both shot-bearing refs are nodes, in the task's own order; the picture-less one is not.
  assert.deepEqual(list(ofKind(shots, "result").map((n) => n.title)), ["Slide", "Other"]);
  assert.deepEqual(list(childIndex(shots).get("task:deck").map((n) => n.id)),
    ["result:views/slide", "result:views/other-slide"]);
  for (const id of ["result:views/slide", "result:views/other-slide"]) {
    const tile = arrange(shots).placed.find((p) => p.node.id === id);
    // A shot is stored 480x270, so a tile draws it at exactly 2x, and a card is that box too.
    assert.deepEqual([tile.w, tile.h], [240, 135], "and every tile is the size of a card");
  }

  // Tiles follow the task's `refs` — newest made first — not the order the view list
  // happens to come back in.
  const reversed = project({
    tasks: [{ ...task("deck"), refs: ["views/other-slide", "views/slide"] }],
    views: [{ view_ref: "views/slide", label: "Slide", shot_url: "/a.png" },
      { view_ref: "views/other-slide", label: "Other", shot_url: "/b.png" }] });
  assert.deepEqual(list(ofKind(reversed, "result").map((n) => n.title)), ["Other", "Slide"]);

  // A picture-less result is never a node, however many there are.
  const duty = project({ tasks: [{ ...task("logs", "serving"), files: Array.from({ length: 40 }, (_, i) => ({ path: `r${i}.json` })) }] });
  assert.equal(ofKind(duty, "result").length, 0);
  // And a task that really does make forty pictures hangs six of them, not forty.
  const many = project({
    tasks: [{ ...task("shoot"), refs: Array.from({ length: 40 }, (_, i) => `views/p${i}`) }],
    views: Array.from({ length: 40 }, (_, i) => ({ view_ref: `views/p${i}`, label: `P${i}`, shot_url: `/p${i}.png` })) });
  assert.equal(ofKind(many, "result").length, 6, "capped below the task");

  const drawnDuty = arrange(duty).placed.find((p) => p.node.id === "task:logs");
  assert.deepEqual([drawnDuty.w, drawnDuty.h], [240, 135], "no picture, same card");
});

test("the air between two nodes is set by where their branches part, not by how deep they sit", () => {
  // The failure this fixes: one gap for every pair meant a task's own results sat exactly as
  // far from it as the next task's results did, so the second rank read as one flat column
  // and only the wires said which branch anything belonged to.
  const views = Array.from({ length: 8 }, (_, i) => ({ view_ref: `views/v${i}`, label: `V${i}`, shot_url: `/v${i}.png` }));
  const tasks = ["a", "b", "c", "d"].map((s, i) => ({ ...task(s), refs: [`views/v${i * 2}`, `views/v${i * 2 + 1}`] }));
  const placed = arrange(project({ tasks, views })).placed;
  const at = (id) => placed.find((p) => p.node.id === id);
  // Two tasks share a side, so their tile columns are adjacent and directly comparable.
  const column = placed.filter((p) => p.node.kind === "result" && p.dir === at("task:a").dir)
    .sort((x, y) => x.y - y.y);
  assert.equal(column.length, 4, "two tasks' pictures stack in one rank on this side");
  const parent = (p) => p.node.id.startsWith("result:views/v0") || p.node.id.startsWith("result:views/v1") ? "first" : "second";
  const gaps = column.slice(1).map((p, i) => ({ gap: p.y - (column[i].y + column[i].h), same: parent(p) === parent(column[i]) }));
  const within = list(gaps.filter((g) => g.same).map((g) => g.gap));
  const across = list(gaps.filter((g) => !g.same).map((g) => g.gap));
  assert.deepEqual(within, [20, 20], "two pictures of one task are a task's gap apart, evenly");
  assert.deepEqual(across, [40], "two pictures whose tasks differ are the core's gap apart, wherever they sit");
});

test("every drawn node has a finite, colored wire and no two boxes overlap", () => {
  const model = project({ tasks: [tagged("a", "Both"), tagged("b", "Both"), task("c", "todo")],
    workers: [worker("unfiled"), worker("linked", "worker", { subject: "a" })] });
  const chart = arrange(model);
  assert.equal(chart.wires.length, chart.placed.length - 1);
  for (const wire of chart.wires) {
    assert.ok(wire.paint); assert.doesNotMatch(`${wire.d} ${wire.paint}`, /NaN|Infinity|undefined/);
  }
  for (const a of chart.placed) for (const b of chart.placed) {
    if (a.node.id >= b.node.id) continue;
    assert.ok(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y, "boxes do not overlap");
  }
});

test("a zoom keeps the point under the pointer where it was, fitted or overflowing", () => {
  const chart = { width: 1600, height: 800 }, frame = { w: 1000, h: 700 };
  const under = (scale, scroll, point) => {
    const { offset } = stage(chart, frame, scale);
    return [(scroll.left + point.x - offset.x) / scale, (scroll.top + point.y - offset.y) / scale];
  };
  // From a centred, fitted drawing into one that overflows both ways, and back out again.
  for (const [from, to, scroll] of [[0.6, 1.5, { left: 0, top: 0 }], [1.5, 0.8, { left: 700, top: 300 }]]) {
    const point = { x: 310, y: 220 };
    const next = zoomAround(chart, frame, from, to, scroll, point);
    const [ax, ay] = under(from, scroll, point), [bx, by] = under(to, next, point);
    assert.ok(Math.abs(ax - bx) < 1e-9 && Math.abs(ay - by) < 1e-9, `${from} -> ${to}`);
  }
  // Half a window of air on every side, whatever the scale.
  assert.deepEqual({ ...stage(chart, frame, 0.5).offset }, { x: 500, y: 350 });
  assert.deepEqual({ ...stage(chart, frame, 2).canvas }, { w: 4200, h: 2300 });
  // The person's range, either side of the 1x Home opens at.
  assert.equal(clampZoom(0.01), 0.25);
  assert.equal(clampZoom(9), 2);
  assert.equal(clampZoom(1), 1);
});

test("layout supports deeper nodes, rather than flattening every row into a hub child", () => {
  const model = project({ tasks: [task("a")] });
  let parent = "task:a";
  for (let i = 0; i < 6; i++) {
    const id = `deep:${i}`;
    model.nodes.push({ id, kind: "result", title: id, sourceRefs: [], data: {} });
    model.edges.push({ id, from: parent, to: id, relation: "contains", primary: true }); parent = id;
  }
  assertConnected(model);
  assert.equal(arrange(model).wires.length, 7);
  assert.equal(childIndex(model).get("deep:4")[0].id, "deep:5");
});

test("every card, the outermost included, can be brought to the middle of the window", () => {
  // The failure: the canvas was the drawing plus only what centring the core needed, so a card
  // on the chart's edge stopped at the window's edge and could not be dragged in to be read.
  for (const count of [0, 1, 3, 20]) {
    const chart = arrange(project({ tasks: Array.from({ length: count }, (_, i) => task(`t${i}`)) }));
    for (const scale of [0.25, 1, 2]) {
      const frame = { w: 1512, h: 850 };
      const { canvas, offset } = stage(chart, frame, scale);
      for (const row of chart.placed) for (const [px, py] of [[row.x, row.y], [row.x + row.w, row.y + row.h]]) {
        const left = offset.x + px * scale - frame.w / 2, top = offset.y + py * scale - frame.h / 2;
        assert.ok(left >= -1e-9 && left <= canvas.w - frame.w + 1e-9, `${row.node.id} x at ${scale}`);
        assert.ok(top >= -1e-9 && top <= canvas.h - frame.h + 1e-9, `${row.node.id} y at ${scale}`);
      }
    }
  }
});

test("a group taken as the centre draws its branch alone, in the colour it has on the whole chart", () => {
  const groups = [{ label: "KNQ", members: ["rollup"], groups: [{ label: "Saima", members: ["closing", "tiers"] }] },
    { label: "Life", members: ["shoes"] }];
  const model = project({ tasks: ["rollup", "closing", "tiers", "shoes", "loose"].map((s) => task(s)), groups,
    workers: [worker("builder", "worker", { subject: "closing" })] });
  const knq = focusOn(model, "group:KNQ");
  assert.equal(knq.rootId, "group:KNQ");
  assert.deepEqual(list(knq.nodes.map((n) => n.id)).sort(),
    ["group:KNQ", "group:Saima", "task:closing", "task:rollup", "task:tiers"]);
  assertConnected({ ...knq, nodes: [{ id: "core" }, ...knq.nodes], edges: [{ from: "core", to: "group:KNQ", primary: true }, ...knq.edges] });
  const whole = branchTones(model), chart = arrange(knq, whole);
  assert.equal(chart.placed[0].node.id, "group:KNQ", "the group stands where the core stood");
  assert.equal(chart.wires.length, chart.placed.length - 1);
  for (const wire of chart.wires) assert.equal(wire.paint, branchPaint(whole.get("group:KNQ")), "still KNQ's colour");
  for (const a of chart.placed) for (const b of chart.placed) {
    if (a.node.id >= b.node.id) continue;
    assert.ok(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y, "boxes do not overlap");
  }
  // A task is not a centre, and a group that is not drawn is the whole chart again.
  assert.equal(focusOn(model, "task:shoes"), null);
  assert.equal(focusOn(model, "group:Gone"), null);
  assert.equal(focusOn(model, null), null);
  // The way back out names every group from the core down.
  assert.deepEqual(list(trail(model, "group:Saima").map((n) => n.id)), ["group:KNQ", "group:Saima"]);
});

test("where the window was is kept as a card and an offset, and survives the chart moving", () => {
  const before = arrange(project({ tasks: ["a", "b", "c"].map((s) => task(s)) }));
  const card = before.placed.find((p) => p.node.id === "task:b");
  const middle = { x: card.x + card.w / 2 + 30, y: card.y + card.h / 2 - 10 };
  const anchor = anchorAt(before, middle);
  assert.deepEqual({ ...anchor }, { id: "task:b", dx: 30, dy: -10 });
  // A day later more work is filed and every branch moves; the same card comes back to the middle.
  const after = arrange(project({ tasks: ["0", "1", "2", "a", "b", "c", "d"].map((s) => task(s)) }));
  const moved = after.placed.find((p) => p.node.id === "task:b");
  assert.deepEqual({ ...anchorPoint(after, anchor) }, { x: moved.x + moved.w / 2 + 30, y: moved.y + moved.h / 2 - 10 });
  // Inside the core's box is the core; a card that is gone is the root's centre.
  const core = after.placed[0];
  assert.equal(anchorAt(after, { x: core.x + 5, y: core.y + 5 }).id, "core");
  assert.deepEqual({ ...anchorPoint(after, { id: "task:closed-since", dx: 400, dy: 400 }) }, { x: core.x + core.w / 2, y: core.y + core.h / 2 });
  assert.deepEqual({ ...anchorPoint(after, undefined) }, { x: core.x + core.w / 2, y: core.y + core.h / 2 });
});

test("finished nodes gradually lose emphasis without making their text invisible", () => {
  const fresh = ofKind(project({ tasks: [task("a", "done", 0)] }), "task")[0];
  const old = ofKind(project({ tasks: [task("a", "done", 23)] }), "task")[0];
  assert.equal(emphasis(fresh, NOW), 1);
  assert.ok(emphasis(old, NOW) < 1 && emphasis(old, NOW) >= 0.78);
});
