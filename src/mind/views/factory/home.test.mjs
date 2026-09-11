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
const { buildHome, arrange, TONE, stateOf, childIndex, revealPath, normalizeSession, emphasis } = runInNewContext(
  `${pure}\n;({ buildHome, arrange, TONE, stateOf, childIndex, revealPath, normalizeSession, emphasis });`,
  { flextree, document: { documentElement: { lang: "en" } }, navigator: { language: "en" } },
);
const NOW = Date.parse("2026-09-11T12:00:00Z");
const hoursAgo = (h) => new Date(NOW - h * 3600000).toISOString();
const task = (subject, status = "doing", hours = 1) => ({ subject, title: subject, status, statusSince: hoursAgo(hours), timeline: [], extra: [], files: [] });
const worker = (id, role = "worker", extra = {}) => ({ run: "0123456789ab", id, role, state: "running", title: `Work ${id}`, started: hoursAgo(2), state_since: hoursAgo(1), ...extra });
const ended = (session, extra = {}) => ({ run: "0123456789ab", session, role: "worker", title: `Work ${session}`, started: hoursAgo(2), ended: hoursAgo(1), how: "closed", ...extra });
const project = (input) => buildHome(input, NOW);
const ofKind = (model, kind) => model.nodes.filter((n) => n.kind === kind);

function assertConnected(model) {
  const ids = new Set(model.nodes.map((n) => n.id));
  assert.equal(ids.size, model.nodes.length, "unique identities");
  const parent = new Map();
  for (const edge of model.edges) {
    assert.ok(ids.has(edge.from) && ids.has(edge.to), "no dangling edge");
    if (!edge.primary) continue;
    assert.ok(!parent.has(edge.to), "exactly one primary parent"); parent.set(edge.to, edge.from);
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

test("all open tasks and all recent completed/cancelled tasks, without result or count gates", () => {
  const tasks = [task("old-open", "doing", 100), task("duty", "serving", 100), task("todo", "todo", 100),
    ...Array.from({ length: 8 }, (_, i) => task(`done-${i}`, "done")), task("cancelled", "cancelled"),
    task("expired", "done", 25), task("expired-cancelled", "cancelled", 25)];
  const model = project({ tasks });
  assert.equal(ofKind(model, "task").length, 12);
  assert.ok(!model.nodes.some((n) => n.title.startsWith("expired")));
  assertConnected(model);
});

test("the boundary is inclusive and uses closure time, not creation or last rewrite", () => {
  const model = project({ tasks: [task("boundary", "done", 24), task("outside", "done", 24.01),
    { ...task("closed-long-ago", "done"), completedAt: hoursAgo(30) },
    { ...task("cancelled-now", "cancelled", 100), cancelledAt: hoursAgo(1) }] });
  assert.deepEqual(Array.from(ofKind(model, "task"), (n) => n.title), ["boundary", "cancelled-now"]);
});

test("missing timestamps stay explicitly unknown instead of silently disappearing", () => {
  const model = project({ tasks: [{ ...task("unknown", "done"), statusSince: null }],
    ended: [ended("crash", { ended: null, how: "restart", started: hoursAgo(100) })] });
  assert.equal(ofKind(model, "task").length, 1);
  assert.equal(ofKind(model, "activity")[0].data.session.endedAt, null);
});

test("core roles aggregate centrally; every other live session is an activity", () => {
  const model = project({ workers: [worker("r", "reaction"), worker("c", "cognition"), worker("f", "reflection"),
    worker("standalone"), worker("idle", "worker", { state: "idle" }), worker("queued", "worker", { state: "waiting" })] });
  assert.equal(model.nodes[0].data.sessions.length, 3);
  assert.equal(ofKind(model, "activity").length, 3);
  assert.equal(model.counts.sessions, 6);
  assertConnected(model);
});

test("session IDs include run, and a live record wins over the same ended record", () => {
  const model = project({ workers: [worker("a")], ended: [ended("a"), ended("a", { run: "abcdef012345" })] });
  assert.equal(ofKind(model, "activity").length, 2);
  assert.equal(ofKind(model, "activity").filter((n) => n.data.session.online).length, 1);
});

test("subject attaches every activity to its task, not its technical owner", () => {
  const model = project({ tasks: [task("goal")], workers: [worker("a", "worker", { subject: "goal", owner: "c" }),
    worker("b", "worker", { subject: "goal", owner: "a" }), worker("c", "cognition")] });
  const links = model.edges.filter((e) => e.relation === "works-on");
  assert.equal(links.length, 2);
  assert.ok(links.every((e) => e.from === "task:goal"));
  assertConnected(model);
});

test("expired task remains as context for retained sessions without changing its status", () => {
  const model = project({ tasks: [task("old", "cancelled", 60)], workers: [worker("a", "worker", { subject: "old" })] });
  const node = ofKind(model, "task")[0];
  assert.equal(node.data.contextOnly, true); assert.equal(node.data.status, "cancelled");
});

test("a missing task or an owner cycle never disconnects a session", () => {
  const model = project({ workers: [worker("a", "worker", { subject: "missing", owner: "b" }), worker("b", "worker", { owner: "a" })] });
  assertConnected(model);
});

test("all recent historical sessions survive, even more than 500", () => {
  const model = project({ ended: [...Array.from({ length: 650 }, (_, i) => ended(`${i}`)), ended("expired", { ended: hoursAgo(25) })] });
  assert.equal(ofKind(model, "activity").length, 650); assertConnected(model);
});

test("idle is live, last-turn failure is not session termination, stale doing is not current", () => {
  const s = normalizeSession(worker("a", "worker", { state: "idle", doing: "old command", last_turn: { outcome: "failed", at: hoursAgo(1) } }), true);
  assert.equal(s.online, true); assert.equal(s.endedAt, null); assert.equal(s.currentAction, null);
  assert.equal(stateOf({ kind: "activity", data: { session: s } }), "failed");
});

test("overview uses public messages and factual transitions, never worker tail or reasoning", () => {
  const model = project({ workers: [worker("c", "cognition", { tail: "PRIVATE THOUGHT", doing: "INTERNAL TOOL" })],
    messages: [{ id: "reply", role: "agent", text: "Earlier plan", ts: hoursAgo(2) },
      { id: "ask", role: "user", text: "Change the plan", ts: hoursAgo(1) }], tasks: [task("delivered", "done")] });
  const overview = ofKind(model, "overview");
  assert.equal(overview.length, 3);
  assert.ok(!JSON.stringify(overview).includes("PRIVATE"));
  assert.ok(!JSON.stringify(overview).includes("INTERNAL"));
  assert.equal(overview.find((n) => n.id === "overview:message:reply").data.freshness, "stale");
  assert.ok(overview.every((n) => n.sourceRefs.length)); assertConnected(model);
});

test("topics are explicit, results are optional children, shared results keep one identity", () => {
  const a = { ...task("a"), extra: [{ key: "project", value: "Home, Product" }], body: "See views/demo/result.jsx" };
  const b = { ...task("b"), body: "See `demo/result`" };
  const model = project({ tasks: [a, b], views: [{ view_ref: "demo/result", label: "Result" }] });
  assert.equal(ofKind(model, "topic").length, 2); assert.equal(ofKind(model, "artifact").length, 1);
  assert.equal(model.edges.filter((e) => e.relation === "produces").length, 2);
  assertConnected(model);
});

test("every visible peripheral node has a finite, colored wire", () => {
  const model = project({ tasks: [task("a", "done"), task("b", "cancelled"), task("c", "todo")],
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

test("collapse is presentation-only and search reveals all ancestors", () => {
  const model = project({ tasks: [{ ...task("a"), extra: [{ key: "project", value: "Home" }] }],
    workers: [worker("b", "worker", { subject: "a" })] });
  const collapsed = new Set(["topic:home", "task:a"]);
  const before = JSON.stringify(model);
  assert.equal(arrange(model, collapsed).placed.length, 2);
  const reveal = revealPath(model, "session:0123456789ab:b", collapsed);
  assert.equal(reveal.size, 0); assert.equal(collapsed.size, 2);
  assert.equal(arrange(model, reveal).placed.length, 4);
  assert.equal(JSON.stringify(model), before);
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
  assert.ok(emphasis(fresh, NOW) > emphasis(old, NOW)); assert.ok(emphasis(old, NOW) >= 0.78);
});
