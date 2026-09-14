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
const { buildHome, arrange, fit, TONE, stateOf, childIndex, normalizeSession, emphasis } = runInNewContext(
  `${pure}\n;({ buildHome, arrange, fit, TONE, stateOf, childIndex, normalizeSession, emphasis });`,
  { flextree, document: { documentElement: { lang: "en" } }, navigator: { language: "en" } },
);
const NOW = Date.parse("2026-09-11T12:00:00Z");
const hoursAgo = (h) => new Date(NOW - h * 3600000).toISOString();
const task = (subject, status = "doing", hours = 1) => ({ subject, title: subject, status, statusSince: hoursAgo(hours), refs: [], extra: [], files: [] });
const tagged = (subject, names, status = "doing") => ({ ...task(subject, status), extra: [{ key: "project", value: names }] });
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

test("core roles aggregate centrally; every other live session is an activity", () => {
  const model = project({ workers: [worker("r", "reaction"), worker("c", "cognition"),
    worker("f", "reflection"), worker("w1"), worker("w2")] });
  assert.equal(model.nodes[0].data.sessions.length, 3);
  assert.equal(ofKind(model, "activity").length, 2);
  assertConnected(model);
});

test("subject attaches every activity to its task, not its technical owner", () => {
  const model = project({ tasks: [task("a"), task("b")],
    workers: [worker("boss", "worker", { subject: "a" }), worker("hand", "worker", { subject: "b", owner: "boss" })] });
  const kids = childIndex(model);
  assert.deepEqual(list(kids.get("task:b").map((n) => n.title)), ["Work hand"]);
  assert.equal(ofKind(model, "activity").find((n) => n.title === "Work hand").data.ownerSessionId, "session:0123456789ab:boss");
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

test("a topic that does not group dissolves, and its task keeps its place", () => {
  // A rank has to earn itself. One topic over one task is a level that groups nothing.
  const thin = project({ tasks: [tagged("a", "Solo")] });
  assert.equal(ofKind(thin, "topic").length, 0);
  assert.deepEqual(list(childIndex(thin).get("core").map((n) => n.id)), ["task:a"]);
  const grouped = project({ tasks: [tagged("a", "Both"), tagged("b", "Both")] });
  assert.deepEqual(titles(grouped, "topic"), ["Both"]);
  assert.equal(childIndex(grouped).get("topic:both").length, 2);
  assertConnected(thin); assertConnected(grouped);
});

test("dissolving a topic never leaves a node with two primary parents", () => {
  // `a` names Kept first (its primary) and Thin second (a reference). Thin groups nothing
  // and dissolves; promoting its reference edge would have made core a second primary.
  const model = project({ tasks: [tagged("a", "Kept,Thin"), tagged("b", "Kept")] });
  assert.deepEqual(titles(model, "topic"), ["Kept"]);
  assertConnected(model);
  assert.equal(childIndex(model).get("topic:kept").length, 2);
});

test("a result is a count on the task, never a card of its own", () => {
  const model = project({
    tasks: [{ ...task("a"), refs: ["views/deck"], files: [{ path: "notes.md" }] }],
    views: [{ view_ref: "views/deck", label: "Deck", shot_url: "/shot.png" }, { view_ref: "views/other" }] });
  assert.equal(model.nodes.length, 2, "core and the task, and nothing else");
  const node = ofKind(model, "task")[0];
  assert.deepEqual(list(node.data.results.map((r) => r.title)), ["Deck", "notes.md"]);
  assert.equal(arrange(model).placed.length, 2);
});

test("a task shows one picture and counts the rest; the picture costs width, never height", () => {
  const shots = project({
    tasks: [{ ...task("deck"), refs: ["views/notes", "views/slide", "views/other-slide"] }],
    views: [{ view_ref: "views/notes", label: "Notes" },
      { view_ref: "views/slide", label: "Slide", shot_url: "/a.png" },
      { view_ref: "views/other-slide", label: "Other", shot_url: "/b.png" }] });
  const deck = ofKind(shots, "task")[0];
  assert.equal(deck.data.results.length, 3, "all three are still counted");
  // The FIRST shot-bearing ref in the task's own order, with no recency claimed.
  const drawn = arrange(shots).placed.find((p) => p.node.id === "task:deck");
  assert.deepEqual([drawn.w, drawn.h], [352, 108], "the picture widens the box and leaves it one card high");
  // The picture on the card is not a node; the ones past it are, one rank below the task.
  assert.deepEqual(list(ofKind(shots, "result").map((n) => n.title)), ["Other"]);
  assert.deepEqual(list(childIndex(shots).get("task:deck").map((n) => n.id)), ["result:views/other-slide"]);
  assert.equal(arrange(shots).placed.find((p) => p.node.id === "result:views/other-slide").h, 76);

  // A mention of one of the app's own surfaces is not an output, so it is neither counted
  // nor eligible to be the picture.
  const mentions = project({
    tasks: [{ ...task("looked"), refs: ["factory/home", "views/real"] }],
    views: [{ view_ref: "factory/home", label: "Home", system: true, shot_url: "/home.png" },
      { view_ref: "views/real", label: "Real", shot_url: "/real.png" }] });
  const looked = ofKind(mentions, "task")[0];
  assert.deepEqual(list(looked.data.results.map((r) => r.title)), ["Real"]);

  // A picture-less result is never a node, however many there are: a filename is a count.
  const duty = project({ tasks: [{ ...task("logs", "serving"), files: Array.from({ length: 40 }, (_, i) => ({ path: `r${i}.json` })) }] });
  assert.equal(ofKind(duty, "result").length, 0);
  assert.equal(ofKind(duty, "task")[0].data.results.length, 40);
  // And a task that really does make forty pictures hangs six of them, not forty.
  const many = project({
    tasks: [{ ...task("shoot"), refs: Array.from({ length: 40 }, (_, i) => `views/p${i}`) }],
    views: Array.from({ length: 40 }, (_, i) => ({ view_ref: `views/p${i}`, label: `P${i}`, shot_url: `/p${i}.png` })) });
  assert.equal(ofKind(many, "result").length, 6, "capped below the task");
  assert.equal(ofKind(many, "task")[0].data.results.length, 40, "while the count still says forty");

  const logs = project({ tasks: [{ ...task("duty", "serving"), files: [{ path: "a.json" }, { path: "b.json" }] }] });
  assert.equal(ofKind(logs, "task")[0].data.results.length, 2);
  const drawnDuty = arrange(logs).placed.find((p) => p.node.id === "task:duty");
  assert.deepEqual([drawnDuty.w, drawnDuty.h], [240, 108], "no picture, no extra width");
});

test("every drawn node has a finite, colored wire and no two boxes overlap", () => {
  const model = project({ tasks: [tagged("a", "Both"), tagged("b", "Both"), task("c", "todo")],
    workers: [worker("unfiled"), worker("linked", "worker", { subject: "a" })] });
  const chart = arrange(model);
  assert.equal(chart.wires.length, chart.placed.length - 1);
  for (const wire of chart.wires) {
    assert.ok(TONE[wire.tone]); assert.doesNotMatch(wire.d, /NaN|Infinity|undefined/);
  }
  for (const a of chart.placed) for (const b of chart.placed) {
    if (a.node.id >= b.node.id) continue;
    assert.ok(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y, "boxes do not overlap");
  }
});

test("the chart is scaled to the window, never enlarged, and never shrunk past legible", () => {
  // Shaped like the instance this was measured on: eleven tasks in hand, six with a picture,
  // four live sessions working on them and four process pictures below their tasks.
  const views = Array.from({ length: 10 }, (_, i) => ({ view_ref: `views/v${i}`, label: `V${i}`, shot_url: `/v${i}.png` }));
  const shot = (i, n = 1) => Array.from({ length: n }, (_, k) => `views/v${i + k}`);
  const tasks = [
    { ...task("t0"), refs: shot(0) }, { ...task("t1"), refs: shot(1, 2) }, { ...task("t2"), refs: shot(3, 3) },
    { ...task("t3"), refs: shot(6, 2) }, { ...task("t4"), refs: shot(8) }, { ...task("t5"), refs: shot(9) },
    task("t6"), task("t7"), task("t8"), task("t9"), task("t10"),
  ];
  const workers = ["t4", "t6", "t8", "t9"].map((subject) => worker(`w-${subject}`, "worker", { subject }));
  const chart = arrange(project({ tasks, workers, views }));
  assert.equal(chart.placed.length, 20);
  // A 14-inch laptop window less the app's own chrome: the whole chart, readable.
  const laptop = fit(chart, { w: 1511, h: 727 });
  assert.ok(chart.width * laptop <= 1511 && chart.height * laptop <= 727, `fits (${chart.width}x${chart.height} at ${laptop})`);
  assert.ok(laptop >= 0.85, `and not by shrinking it to a thumbnail (${laptop})`);
  assert.equal(fit(chart, { w: 4000, h: 3000 }), 1, "a big window does not enlarge it");
  assert.equal(fit(chart, { w: 800, h: 400 }), 0.7, "a small one scrolls rather than go illegible");
});

test("layout supports deeper nodes, rather than flattening every row into a hub child", () => {
  const model = project({ tasks: [task("a")] });
  let parent = "task:a";
  for (let i = 0; i < 6; i++) {
    const id = `deep:${i}`;
    model.nodes.push({ id, kind: "topic", title: id, sourceRefs: [], data: {} });
    model.edges.push({ id, from: parent, to: id, relation: "contains", primary: true }); parent = id;
  }
  assertConnected(model);
  assert.equal(arrange(model).wires.length, 7);
  assert.equal(childIndex(model).get("deep:4")[0].id, "deep:5");
});

test("finished nodes gradually lose emphasis without making their text invisible", () => {
  const fresh = ofKind(project({ tasks: [task("a", "done", 0)] }), "task")[0];
  const old = ofKind(project({ tasks: [task("a", "done", 23)] }), "task")[0];
  assert.equal(emphasis(fresh, NOW), 1);
  assert.ok(emphasis(old, NOW) < 1 && emphasis(old, NOW) >= 0.78);
});
