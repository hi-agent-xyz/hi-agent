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
const { buildHome, arrange, stage, zoomAround, clampZoom, TONE, stateOf, childIndex, normalizeSession, emphasis } = runInNewContext(
  `${pure}\n;({ buildHome, arrange, stage, zoomAround, clampZoom, TONE, stateOf, childIndex, normalizeSession, emphasis });`,
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

test("every task is its own branch off the core, whatever systems it touches", () => {
  // `systems` names the records a task touches, not what it belongs to: a birthday deck
  // whose photos came over Feishu is not part of a Feishu project. Nothing groups tasks.
  const model = project({ tasks: [tagged("deck", "feishu, wecom"), tagged("brief", "feishu"), task("plain")] });
  assert.deepEqual(list(childIndex(model).get("core").map((n) => n.id)).sort(), ["task:brief", "task:deck", "task:plain"]);
  assert.deepEqual(list(new Set(model.nodes.map((n) => n.kind))).sort(), ["core", "task"]);
  assertConnected(model);
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

  // A mention of one of the app's own surfaces is not an output, so it is never the picture.
  const mentions = project({
    tasks: [{ ...task("looked"), refs: ["factory/home", "views/real"] }],
    views: [{ view_ref: "factory/home", label: "Home", system: true, shot_url: "/home.png" },
      { view_ref: "views/real", label: "Real", shot_url: "/real.png" }] });
  const looked = ofKind(mentions, "task")[0];
  assert.deepEqual(list(looked.data.results.map((r) => r.title)), ["Real"]);

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
    assert.ok(TONE[wire.tone]); assert.doesNotMatch(wire.d, /NaN|Infinity|undefined/);
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
  // A drawing smaller than the window is centred in it; one larger starts at the origin.
  assert.deepEqual({ ...stage(chart, frame, 0.5).offset }, { x: 100, y: 150 });
  assert.deepEqual({ ...stage(chart, frame, 2).offset }, { x: 0, y: 0 });
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

test("finished nodes gradually lose emphasis without making their text invisible", () => {
  const fresh = ofKind(project({ tasks: [task("a", "done", 0)] }), "task")[0];
  const old = ofKind(project({ tasks: [task("a", "done", 23)] }), "task")[0];
  assert.equal(emphasis(fresh, NOW), 1);
  assert.ok(emphasis(old, NOW) < 1 && emphasis(old, NOW) >= 0.78);
});
