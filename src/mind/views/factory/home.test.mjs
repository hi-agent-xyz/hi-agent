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
const { buildHome, arrange, stage, zoomAround, clampZoom, TONE, stateOf, runningHand, nodeTime, childIndex, normalizeSession, emphasis, groupIcon, branchHues, branchTones, branchPaint, focusOn, trail, budgeted, opening, heat, carry, drawn, glide, boxAt } = runInNewContext(
  `${pure}\n;({ buildHome, arrange, stage, zoomAround, clampZoom, TONE, stateOf, runningHand, nodeTime, childIndex, normalizeSession, emphasis, groupIcon, branchHues, branchTones, branchPaint, focusOn, trail, budgeted, opening, heat, carry, drawn, glide, boxAt });`,
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
/**
 * The rows this model calls **in hand** — what is going on now, as against what a branch has
 * been through. Every row in the ledger but a spent cancellation is a node whatever its answer
 * here (`titles` is the whole of them); this is the tier that decides which ones fill the
 * window first, which ones a group counts as *N more*, and which closures are news on the core.
 */
const handed = (model) => list(ofKind(model, "task").filter((n) => n.data.inHand).map((n) => n.title)).sort();
const handCount = (model) => ofKind(model, "task").filter((n) => n.data.inHand).length;

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

test("what is open is in hand however old it is; what is closed answers to the work, not the clock", () => {
  const model = project({ tasks: [
    task("stale-but-open", "todo", 900), task("serving", "serving", 900),
    task("just-closed", "done", 3), task("closed-last-week", "done", 200),
  ], messages: [{ id: "m", role: "user", text: "ok", ts: hoursAgo(150) }] });
  assert.deepEqual(handed(model), ["just-closed", "serving", "stale-but-open"]);
  // **And the one that is not in hand is still a node.** The model is the whole ledger with
  // the whole of its structure; what the clock decides is rank, so a lens — a group taken as
  // the centre, or simply a window with room left — can still reach it.
  assert.deepEqual(titles(model, "task"),
    ["closed-last-week", "just-closed", "serving", "stale-but-open"]);
  assertConnected(model);
});

test("the ceiling is inclusive and uses closure time, not creation or last rewrite", () => {
  const at = (h) => project({ tasks: [{ ...task("a", "done", 0), completedAt: hoursAgo(h), statusSince: hoursAgo(0) }] });
  assert.equal(handCount(at(167)), 1);
  assert.equal(handCount(at(169)), 0);
  assert.equal(ofKind(at(169), "task").length, 1, "past the ceiling is history, not gone");
});

test("a closed row is kept until the person has been back, and then not long", () => {
  // A notice is kept for a glance the person actually gets. The fixed window could not promise
  // that — work closing at 3am was a day old by the time anyone looked, and work closing
  // mid-conversation held a card for the next 23 hours.
  const closed = (h, inbound) => project({ tasks: [task("a", "done", h)],
    messages: inbound.map((t, i) => ({ id: `m${i}`, role: "user", text: "ok", ts: hoursAgo(t) })) });
  assert.equal(handCount(closed(6, [])), 1, "nobody has been back: still a notice");
  assert.equal(handCount(closed(6, [8])), 1, "they spoke BEFORE it closed; that is not collecting it");
  assert.equal(handCount(closed(6, [0.5])), 1, "collected half an hour ago: inside the grace");
  assert.equal(handCount(closed(6, [2])), 0, "collected two hours ago: the grace has run out");
  // The grace is the FIRST message after it closed, not the most convenient one: a person who
  // came back four hours ago and has kept talking has had it in front of them the whole time.
  assert.equal(handCount(closed(6, [4, 3, 0.1])), 0, "collected four hours ago, still talking");
});

test("the grace runs from this row's own collection, never from the newest message", () => {
  // Measuring it from the last inbound message lets one message resurrect every closed row at
  // once: on the instance this was measured against, a message 0.3h old turned 14 closed cards
  // into 37, the oldest of them seven days closed.
  const model = project({ tasks: [task("ancient", "done", 120), task("fresh", "done", 0.2)],
    messages: [{ id: "m0", role: "user", text: "ok", ts: hoursAgo(100) },
      { id: "m1", role: "user", text: "ok", ts: hoursAgo(0.1) }] });
  assert.deepEqual(handed(model), ["fresh"]);
});

test("a thread still in hand keeps its closed rows; a finished one does not", () => {
  // The person's own words for it: the parent has not disappeared. The innermost group is that
  // parent — testing the first-level branch instead would have kept a merged-video row whose
  // own group was finished, because the client branch above it was busy.
  const groups = [{ label: "client", members: [], groups: [
    { label: "still-going", members: ["report", "open-work"] },
    { label: "finished", members: ["merged-video"] } ] },
    { label: "elsewhere", members: ["open-work-2"] }];
  const model = project({ groups, tasks: [
    task("open-work", "doing", 1), task("open-work-2", "doing", 1),
    task("report", "done", 30), task("merged-video", "done", 30),
  ], messages: [{ id: "m", role: "user", text: "ok", ts: hoursAgo(20) }] });
  assert.deepEqual(handed(model), ["open-work", "open-work-2", "report"]);
  assertConnected(model);
});

test("a cancellation is on Home for an hour, and then not on Home at all", () => {
  // It has nothing to come back to: what it made on the way is process, and the row's own word
  // says the work is not happening. Ranked as history, two of these three days closed were
  // drawn in a group of five duties because the window had room, and read as pure interference.
  const cancelled = (subject, minutesAgo, extra = {}) => ({ ...task(subject, "cancelled", 0),
    cancelledAt: new Date(NOW - minutesAgo * 60000).toISOString(), refs: [`shot-${subject}`], ...extra });
  const groups = [{ label: "duties", members: ["on-duty", "just-dropped", "dropped"] },
    { label: "gone", members: ["dropped-too"] }];
  const model = project({ groups,
    views: ["just-dropped", "dropped"].map((s) => ({ view_ref: `shot-${s}`, label: s, shot_url: `/${s}.png` })),
    workers: [worker("still-on-it", "worker", { subject: "dropped" })],
    tasks: [task("on-duty", "serving", 1), cancelled("just-dropped", 50), cancelled("dropped", 70),
      cancelled("dropped-too", 3 * 24 * 60), { ...task("no-stamp", "cancelled", 0), statusSince: null }] });
  // Inside its hour it is a notice: in hand beside a live sibling, and news on the core.
  assert.deepEqual(titles(model, "task"), ["just-dropped", "on-duty"]);
  assert.deepEqual(handed(model), ["just-dropped", "on-duty"]);
  assert.ok(model.nodes.some((n) => n.id === "overview:task:task:just-dropped"));
  // Past it, gone — not history. A live sibling, nobody being back yet and a picture of its own
  // keep nothing; a group holding nothing else is not added; a missing time reads as past it.
  assert.deepEqual(ids(model, "result"), ["result:shot-just-dropped"]);
  assert.ok(!model.nodes.some((n) => n.id === "group:gone"));
  // A live session on it is still live work: it stands on the core under its own title.
  const hand = ofKind(model, "activity")[0];
  assert.equal(model.edges.find((e) => e.primary && e.to === hand.id).from, "core");
  // And pressed into the group, with the window to itself, it is still not drawn.
  assert.deepEqual(ids(budgeted(focusOn(model, "group:duties"), LAPTOP).model, "task").sort(),
    ["task:just-dropped", "task:on-duty"]);
  assertConnected(model);
});

test("a thread keeps a closed row past the fade, and the fade no longer decides retention", () => {
  const groups = [{ label: "g", members: ["report", "open-work"] }];
  const model = project({ groups, tasks: [task("open-work", "doing", 1), task("report", "done", 100)],
    messages: [{ id: "m", role: "user", text: "ok", ts: hoursAgo(90) }] });
  assert.deepEqual(handed(model), ["open-work", "report"]);
  // Dimmest, but drawn and readable: emphasis is about age, retention is about the work.
  const node = model.nodes.find((n) => n.id === "task:report");
  assert.equal(emphasis(node, NOW), 0.78);
});

test("an unknown closure time reads as old, not as timeless", () => {
  // The inversion this surface was rebuilt for. A record with no closure stamp used to be
  // retained forever on the rule that a missing time must not be fabricated; on a real
  // instance that was nine permanent residents whose records were over a month stale.
  const model = project({ tasks: [{ ...task("a", "done"), completedAt: null, statusSince: null }] });
  assert.equal(handCount(model), 0);
  // Still never fabricated for something that IS shown: an open task with no stamp stays.
  const open = project({ tasks: [{ ...task("b", "doing"), statusSince: null }] });
  assert.equal(handCount(open), 1);
  assert.equal(open.nodes.find((n) => n.kind === "task").data.endedAt, null);
});

test("a live session does not re-admit its own expired task", () => {
  // The single biggest leak in the surface this replaced: any recent session naming a task
  // kept that task drawn as a peer of the work in hand, however long ago it had closed.
  const model = project({ tasks: [task("old", "done", 900)], workers: [worker("w", "worker", { subject: "old" })] });
  assert.equal(handCount(model), 0, "the row is history, and a live hand does not change that");
  const activity = ofKind(model, "activity")[0];
  assert.equal(activity.title, "Work w");
  assert.equal(activity.data.taskId, null);
  assertConnected(model);
});

test("an ended session is not this surface's subject at all", () => {
  const model = project({ workers: [worker("live")], ended: [{ run: "0123456789ab", session: "gone", ended: hoursAgo(1) }] });
  assert.deepEqual(titles(model, "activity"), ["Work live"]);
});

test("conversation and coordination make the core; two hands on a task are cards, one is not", () => {
  const model = project({ tasks: [task("a")], workers: [worker("r", "reaction"), worker("c", "cognition"),
    worker("f", "reflection"), worker("w1", "worker", { subject: "a" }), worker("w2", "worker", { subject: "a" })] });
  assert.deepEqual(list(model.nodes[0].data.sessions.map((s) => s.role)).sort(), ["cognition", "reaction"]);
  const drawn = ofKind(model, "task")[0];
  assert.deepEqual(list(drawn.data.sessions.map((s) => s.slug)).sort(), ["w1", "w2"]);
  // Two hands on the row, so both are cards on its branch — each keeping its own errand's title.
  assert.deepEqual(titles(model, "activity"), ["Review", "Work w1", "Work w2"]);
  assert.deepEqual(list(childIndex(model).get("task:a").map((n) => n.id)),
    ["session:0123456789ab:w1", "session:0123456789ab:w2"]);
  assert.ok(model.edges.some((e) => e.from === "task:a" && e.to === "session:0123456789ab:w1" && e.relation === "works-on"));
  assert.ok(drawn.sourceRefs.some((r) => r.kind === "session"), "the sessions stay source references");
  assertConnected(model);
  // One hand draws no card: its title is the task's said back, so the card would be a
  // restatement costing a node. The one thing it had to add is a word on the row instead.
  const alone = project({ tasks: [task("a")], workers: [worker("w1", "worker", { subject: "a" })] });
  assert.deepEqual(titles(alone, "activity"), []);
  assert.deepEqual(list(childIndex(alone).get("task:a")), []);
  assert.deepEqual(list(ofKind(alone, "task")[0].data.sessions.map((s) => s.slug)), ["w1"], "it is still the row's own state");
  assert.equal(stateOf(ofKind(alone, "task")[0]), "running", "and the row says it is in a turn");
});

test("a task's word is the ledger's, except that a hand in a turn makes it Working", () => {
  // The grid the two-line card promised mostly does not exist: nothing is being worked on while
  // it is still to do, and a closed row is closed whatever is still warm beside it. What is
  // left of it is one word — `In progress` is where the row got to, and it was read as saying
  // that something is happening in it, which it never did.
  const of = (status, workers, hours = 1) => ofKind(project({ tasks: [task("a", status, hours)], workers }), "task")[0];
  const on = (state, extra = {}) => worker(`w${state}`, "worker", { subject: "a", state, ...extra });
  assert.equal(stateOf(of("todo", [])), "todo");
  assert.equal(stateOf(of("doing", [])), "doing", "open, and nobody on it");
  assert.equal(stateOf(of("doing", [on("idle")])), "doing", "a hand that is not in a turn is not working");
  assert.equal(stateOf(of("doing", [on("running")])), "running");
  assert.equal(stateOf(of("doing", [on("idle"), on("running")])), "running", "any hand in a turn");
  assert.equal(stateOf(of("serving", [on("idle")])), "serving");
  assert.equal(stateOf(of("serving", [on("running")])), "running", "a duty in a turn spends On duty");
  assert.equal(stateOf(of("done", [on("running")])), "done", "a closed row is closed whatever is still warm");
  // Not a cut turn: that is one hand's state, and that hand has a card to wear it on.
  const cut = of("doing", [on("idle", { last_turn: { outcome: "failed" } })]);
  assert.equal(stateOf(cut), "doing");
  const cards = ofKind(project({ tasks: [task("a")], workers: [on("idle", { last_turn: { outcome: "failed" } }), on("running")] }), "activity");
  assert.equal(stateOf(cards.find((n) => n.data.session.state === "idle")), "failed");
  assert.equal(TONE.failed, "var(--danger)");
});

test("the clock is the age of whatever the word says", () => {
  const of = (status, workers, hours = 1) => ofKind(project({ tasks: [task("a", status, hours)], workers }), "task")[0];
  const on = (state, extra = {}) => worker(`w${state}`, "worker", { subject: "a", state, ...extra });
  // A duty open for 15 days, in a turn for one hour, reads `Working · 1h ago`: the row's own
  // clock there would have said 15d and read as fifteen days of work.
  assert.equal(nodeTime(of("serving", [on("running", { state_since: hoursAgo(1) })], 360)), hoursAgo(1));
  // Two hands in a turn: how long the row has had something in flight, not when the last one
  // happened to start.
  const both = of("doing", [on("running", { state_since: hoursAgo(1) }),
    { ...worker("w2", "worker", { subject: "a", state: "running", state_since: hoursAgo(3) }) }], 360);
  assert.equal(nodeTime(both), hoursAgo(3));
  // With nothing in a turn it is the ledger's again: how long the row has held its status.
  assert.equal(nodeTime(of("serving", [on("idle", { state_since: hoursAgo(1) })], 360)), hoursAgo(360));
  assert.equal(runningHand(of("serving", [on("idle")])), null);
});

test("code draws no group of its own: a session on no drawn task is a card on the core", () => {
  // The only groups are the arrangement's. A built-in one for the agent's own upkeep drew one
  // category under two headings for a person whose arrangement already had a group for it.
  const model = project({ tasks: [task("a")], groups: [{ label: "Work", members: ["a"] }],
    workers: [worker("reflection", "reflection"), worker("sweep", "worker", { type: "task-manager", owner: "cognition" }),
      worker("builder", "worker", { subject: "a" }), worker("orphan", "worker", { subject: "aged-out" })] });
  const kids = childIndex(model);
  assert.deepEqual(list(ofKind(model, "group").map((n) => n.id)), ["group:Work"]);
  assert.deepEqual(list(ofKind(model, "task")[0].data.sessions.map((s) => s.slug)), ["builder"], "a session on a drawn task is the task's");
  assert.deepEqual(list(kids.get("core").filter((n) => n.kind === "activity").map((n) => n.id)).sort(),
    ["session:0123456789ab:orphan", "session:0123456789ab:reflection", "session:0123456789ab:sweep"]);
  assert.equal(model.nodes.find((n) => n.id === "session:0123456789ab:reflection").title, "Review");
  // Each is a branch of its own, coloured like any card in no group.
  const tones = branchTones(model);
  assert.notEqual(tones.get("session:0123456789ab:sweep"), undefined);
  assert.notEqual(tones.get("session:0123456789ab:sweep"), tones.get("group:Work"));
  assertConnected(model);
});

test("the group the arrangement marks for the agent's own upkeep holds it, beside its own tasks", () => {
  // One category, one heading: the person's group for hi-agent's own upkeep holds both the
  // changes to hi-agent they asked for and the sessions nobody asked for.
  const workers = [worker("reflection", "reflection"), worker("sweep", "worker", { type: "task-manager" }),
    worker("orphan", "worker", { subject: "aged-out" })];
  const model = project({ tasks: [task("a"), task("icons")], workers, groups: [
    { label: "Work", members: ["a"] },
    { label: "自身维护", members: ["icons"], upkeep: true },
  ] });
  const kids = childIndex(model);
  assert.deepEqual(list(ofKind(model, "group").map((n) => n.id)), ["group:Work", "group:自身维护"]);
  assert.deepEqual(list(kids.get("group:自身维护").map((n) => n.id)),
    ["task:icons", "session:0123456789ab:reflection", "session:0123456789ab:sweep"]);
  assert.ok(kids.get("core").some((n) => n.id === "session:0123456789ab:orphan"), "somebody's aged-out work is not upkeep");
  assert.equal(branchTones(model).get("session:0123456789ab:sweep"), branchTones(model).get("group:自身维护"));
  assertConnected(model);
  // With none of its tasks drawn it is still drawn on the way to the upkeep it holds, and not
  // at all once nothing it holds is live.
  const alone = (live) => project({ tasks: [], workers: live, groups: [{ label: "自身维护", members: ["gone"], upkeep: true }] });
  assert.deepEqual(list(childIndex(alone(workers)).get("group:自身维护").map((n) => n.id)),
    ["session:0123456789ab:reflection", "session:0123456789ab:sweep"]);
  assert.equal(alone([]).nodes.some((n) => n.kind === "group"), false);
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
  // `shipped` closed after the last thing the person said, so nobody has collected it yet and
  // it is still drawn — this test is about what the overview may carry, not about retention.
  const model = project({ tasks: [task("shipped", "done", 0.25)], workers: [worker("w", "worker", { doing: "internal narration" })],
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
  assert.deepEqual(list(ofKind(model, "task")[0].data.sessions.map((s) => s.slug)), ["w1"],
    "the lone session is the card's own state, and never a peer of it on the core");
  assert.deepEqual(list(children.get("task:kt8-046")), []);
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

test("the model carries a finished branch whole, and a member naming no row is nothing anywhere", () => {
  const model = project({
    tasks: [task("a"), task("closed-long-ago", "done", 900)],
    groups: [{ label: "Kept", members: ["a"] },
      { label: "Outer", groups: [{ label: "Inner", members: ["closed-long-ago", "never-existed"] }] }] });
  // The row is history and its two groups are still structure: the whole relationship is here,
  // and whether any of it is drawn is the window's question, below.
  assert.deepEqual(titles(model, "group"), ["Inner", "Kept", "Outer"]);
  assert.deepEqual(titles(model, "task"), ["a", "closed-long-ago"]);
  // The ledger says what exists; a member naming no row is nothing, here or anywhere.
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
  assert.equal(card.x - (group.x + group.w), 56, "a narrow heading still leaves the wire its bend");
});

test("the gutter off the core is the widest, because the whole fan bends in it", () => {
  const model = project({ tasks: [task("kt8-046")],
    groups: [{ label: "KTV", members: ["kt8-046"] }] });
  const placed = arrange(model).placed;
  const core = placed.find((p) => p.node.kind === "core");
  const group = placed.find((p) => p.node.id === "group:KTV");
  const card = placed.find((p) => p.node.id === "task:kt8-046");
  const gutter = (from, to) => (to.dir > 0 ? to.x - (from.x + from.w) : from.x - (to.x + to.w));
  assert.equal(gutter(core, group), 104, "every group parts from the core, so that gutter carries the whole fan");
  assert.equal(gutter(group, card), 56, "a deeper rank fans two or three ways, and needs less");
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

test("a wire is its branch's one colour at every depth, a card in no group included, and never the status", () => {
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

  // A card in no group is a branch of its own: a hue of its own, and none of a group's.
  const loose = tones.get("task:loose");
  assert.notEqual(loose, undefined);
  assert.ok(![knq, tones.get("group:学习类")].includes(loose), "no group's colour");
  assert.equal(arrange(model).wires.find((w) => w.to === "task:loose").paint, branchPaint(loose));
  // Groups are handed theirs first, so ungrouped cards coming and going move none of them.
  const busier = branchTones(project({ ...input, tasks: [...input.tasks, ...["a", "b", "c", "d", "e", "f", "g"].map((s) => task(s))] }));
  for (const id of ["group:KNQ", "group:学习类"]) assert.equal(busier.get(id), tones.get(id), `${id} keeps its hue`);

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
    // A shot is stored 960x540, so a tile has device pixels to spare at full zoom, and a
    // card is that box too.
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

test("an update glides what stays from where it was on screen, and fades what goes where it stood", () => {
  const frame = { w: 1400, h: 780 };
  // Where a box is on screen: the one thing a glide has to keep continuous.
  const screen = (box, at) => [at.offset.x + box.x * at.scale - at.scroll.left,
    at.offset.y + box.y * at.scale - at.scroll.top, box.w * at.scale, box.h * at.scale];
  const near = (a, b, why) => assert.ok(a.every((v, i) => Math.abs(v - b[i]) < 1e-6), `${why}: ${a} vs ${b}`);
  // Any two stages: a carried box sits exactly where it sat, at the new scale and scroll.
  const box = { x: 120, y: 48, w: 240, h: 135, dir: 1 };
  const from = { scale: 0.83, offset: { x: 700, y: 390 }, scroll: { left: 610, top: 222 } };
  const to = { scale: 1, offset: { x: 700, y: 390 }, scroll: { left: 480, top: 300 } };
  near(screen(carry(box, from, to), to), screen(box, from), "carried");
  // A real update: one group's only task goes, and Home refits and recentres around what is left.
  const groups = [{ label: "A", members: ["a1", "a2"] }, { label: "B", members: ["b1"] }];
  const tasks = ["a1", "a2", "b1", "loose1", "loose2"].map((s) => task(s));
  const before = arrange(project({ tasks, groups }));
  const after = arrange(project({ tasks: tasks.filter((t) => t.subject !== "b1"), groups }));
  const centred = (chart, scale) => ({ scale, offset: stage(chart, frame, scale).offset,
    scroll: { left: (chart.width * scale) / 2, top: (chart.height * scale) / 2 } });
  const a = centred(before, 0.9), b = centred(after, 1);
  const was = new Map(before.placed.map((row) => [row.node.id, { row, box: row }]));
  const plan = glide(was, a, b, after);
  assert.deepEqual(list(plan.leaving.keys()).sort(), ["group:B", "task:b1"]);
  for (const [id, gone] of plan.leaving) {
    near(screen(gone.box, b), screen(was.get(id).box, a), `${id} fades where it stood`);
    assert.equal(gone.row, was.get(id).row, `${id} keeps the row it is rendered at`);
  }
  assert.ok(plan.moves.size > 0, "a refit moves what stays");
  for (const [id, move] of plan.moves) {
    near(screen(move.from, b), screen(was.get(id).box, a), `${id} starts where it was`);
    assert.equal(move.to, after.placed.find((row) => row.node.id === id), `${id} ends on its new row`);
  }
  // Nothing moved, nothing left: an update that changes nothing on screen plans nothing.
  const still = glide(new Map(after.placed.map((row) => [row.node.id, { row, box: row }])), b, b, after);
  assert.equal(still.moves.size + still.leaving.size, 0);
  // A card that changes sides hops: on its old side until halfway, on its new one after, never
  // anywhere between — between is across the core. One that stays on its side glides.
  const [crosser, stayer] = after.placed.filter((row) => row.dir !== 0);
  const moved = (row, dir) => [row.node.id, { row: { ...row, dir }, box: { ...row, y: row.y + 300, dir } }];
  const sides = glide(new Map([moved(crosser, -crosser.dir), moved(stayer, stayer.dir)]), b, b, after);
  const hop = sides.moves.get(crosser.node.id);
  assert.equal(hop.hop, true);
  assert.equal(sides.moves.get(stayer.node.id).hop, false);
  assert.equal(boxAt(hop, 0.49), hop.from);
  assert.equal(boxAt(hop, 0.51), hop.to);
  assert.equal(boxAt(sides.moves.get(stayer.node.id), 0.5).y, stayer.y + 150);
  // A glide keeps a card's shape: a same-shaped box is drawn exactly, and a group growing into the
  // centre is scaled alike on both axes about the box it is growing into.
  near(Object.values(drawn(box, { ...box, x: 0, w: 480, h: 270 })).slice(0, 5), [0, 48, 480, 270, 2], "same shape");
  const grown = drawn({ x: 0, y: 0, w: 144, h: 56 }, { x: 0, y: 0, w: 240, h: 80 });
  near([grown.w / grown.h, grown.x + grown.w / 2, grown.y + grown.h / 2], [144 / 56, 120, 40], "shape and centre");
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

const LAPTOP = { w: 1512, h: 855 };
const ids = (model, kind) => list(model.nodes.filter((n) => n.kind === kind).map((n) => n.id));
/** Two groups of `n` tasks each, every task a different age so heat has one answer. */
function twoGroups(n) {
  const tasks = [], a = [], b = [];
  for (let i = 0; i < n; i++) {
    tasks.push(task(`a${i}`, "doing", 1 + i * 2), task(`b${i}`, "doing", 2 + i * 2));
    a.push(`a${i}`); b.push(`b${i}`);
  }
  return project({ tasks, groups: [{ label: "A", members: a }, { label: "B", members: b }] });
}

test("the chart keeps every group and cuts only cards, hottest first, until the window at the overview scale is full", () => {
  // A day that does not fit: 16 cards in two groups is far taller than a laptop window at 0.8.
  const model = twoGroups(8), cut = budgeted(model, LAPTOP);
  assert.deepEqual(ids(cut.model, "group").sort(), ["group:A", "group:B"], "no group is ever cut");
  const drawn = ids(cut.model, "task");
  assert.ok(drawn.length > 2 && drawn.length < 16, `some cards cut, some kept (${drawn.length})`);
  const chart = arrange(cut.model);
  assert.ok(chart.width <= LAPTOP.w / 0.8 && chart.height <= LAPTOP.h / 0.8, "the whole fits the window at the overview scale");
  // A group's cards cost the same and stack on one side, so what it draws is its freshest ones.
  for (const g of ["a", "b"]) {
    const kept = drawn.filter((id) => id.startsWith(`task:${g}`)).map((id) => Number(id.slice(6))).sort((x, y) => x - y);
    assert.deepEqual(kept, kept.map((_, i) => i), `${g}'s drawn cards are its hottest`);
  }
  // Whatever is put away is counted on the group it hangs off, so the group says it holds more.
  assert.equal(cut.hidden.get("group:A") + cut.hidden.get("group:B"), 16 - drawn.length);
  // A day that fits is drawn whole, and nothing is counted.
  const small = budgeted(twoGroups(2), LAPTOP);
  assert.equal(ids(small.model, "task").length, 4);
  assert.equal(small.hidden.size, 0);
});

test("ungrouped work competes like any other, and one waiting on the person comes before fresher work", () => {
  // Someone who has never grouped anything has only ungrouped cards. Exempting them meant nothing
  // was ever cut for that person — and, watched, nineteen closed ones left every group bare.
  const loose = budgeted(project({ tasks: Array.from({ length: 30 }, (_, i) => task(`loose${i}`, "doing", 1 + i)) }), LAPTOP);
  const kept = ids(loose.model, "task");
  assert.ok(kept.length > 2 && kept.length < 30, `cut to the window (${kept.length})`);
  assert.deepEqual(kept.map((id) => Number(id.slice("task:loose".length))).sort((a, b) => a - b),
    kept.map((_, i) => i), "and what stays is the freshest");
  assert.equal(loose.hidden.get("core"), 30 - kept.length, "the core counts what it put away");
  // In a group that draws one or two, the two-day-old row that is waiting on him is one of them.
  const members = Array.from({ length: 10 }, (_, i) => `w${i}`);
  const tasks = members.map((s, i) => task(s, "doing", 1 + i));
  tasks[9] = { ...task("w9", "doing", 50), latest: { kind: "waiting", at: hoursAgo(50), text: "which one?" } };
  const others = Array.from({ length: 10 }, (_, i) => task(`o${i}`, "doing", 1 + i));
  const cut = budgeted(project({ tasks: [...tasks, ...others],
    groups: [{ label: "W", members }, { label: "O", members: others.map((t) => t.subject) }] }), LAPTOP);
  const drawn = ids(cut.model, "task");
  assert.ok(drawn.includes("task:w9"), "the waiting row is drawn");
  assert.ok(!drawn.includes("task:w8"), "ahead of a fresher row that is not");
});

test("pictures take only the width the cards left, so one cannot keep a card out of its inner group", () => {
  // The failure: offered with its card, a picture on one side took the width a card on the other
  // side needed to reach its inner group, and the branch drew an older card instead. `Study`'s
  // task is the hottest and has a picture; `Life` holds a warm card inside `Shoes` and a cold one
  // on its own. In a window whose room fits an inner group and a picture, but not both at once:
  const views = [{ view_ref: "views/p", label: "P", shot_url: "/p.png" }];
  const model = project({ views, tasks: [
    { ...task("t0", "doing", 1), refs: ["views/p"] }, task("s0", "doing", 3), task("l0", "doing", 9)],
    groups: [{ label: "Life", members: ["l0"], groups: [{ label: "Shoes", members: ["s0"] }] }, { label: "Study", members: ["t0"] }] });
  const both = arrange(model), narrow = { w: (both.width - 40) * 0.8, h: LAPTOP.h };
  const cut = budgeted(model, narrow);
  assert.ok(ids(cut.model, "task").includes("task:s0"), "the warm card reaches its inner group");
  assert.deepEqual(ids(cut.model, "result"), [], "and the picture is what gives way");
  // With room for both, both are drawn.
  assert.deepEqual(ids(budgeted(model, { w: (both.width + 40) * 0.8, h: LAPTOP.h }).model, "result"), ["result:views/p"]);
});

test("what is drawn keeps the record's order: heat decides whether a card is on the chart, never where", () => {
  const model = twoGroups(8), cut = budgeted(model, LAPTOP);
  const order = (m) => list((childIndex(m).get("group:A") || []).map((n) => n.id));
  const kept = new Set(ids(cut.model, "task"));
  assert.deepEqual(order(cut.model), order(model).filter((id) => kept.has(id)));
});

test("the window opens on the whole drawing, never above 1x and never below the overview scale", () => {
  const frame = { w: 1512, h: 855 };
  assert.equal(opening({ width: 600, height: 400 }, frame), 1, "a small day is not blown up");
  assert.equal(opening({ width: 1512 / 0.9, height: 855 / 0.9 }, frame).toFixed(3), "0.900", "a day that fits at 0.9 opens whole");
  // What may not be cut can still overflow — a group's own cards once it is the centre. That
  // scrolls at the overview scale; it does not shrink to whatever the day forces.
  assert.equal(opening({ width: 1512 / 0.3, height: 855 }, frame), 0.8);
});

test("a group is a heading on the way to a drawn card, or because it holds work in hand", () => {
  const finished = { label: "Outer", groups: [{ label: "Inner", members: ["closed-long-ago"] }] };
  const quiet = project({ tasks: [task("a"), task("closed-long-ago", "done", 900)],
    groups: [{ label: "Kept", members: ["a"] }, finished] });
  // A window with the room for it draws the history, and then the headings on the way to it.
  assert.deepEqual(ids(budgeted(quiet, LAPTOP).model, "group").sort(),
    ["group:Inner", "group:Kept", "group:Outer"]);
  // A window with no room for it draws neither the row nor the two headings above it. An empty
  // heading is structure standing where its content used to be, which is what this refuses.
  const busy = project({
    tasks: [...Array.from({ length: 16 }, (_, i) => task(`w${i}`, "doing", 1 + i)),
      task("closed-long-ago", "done", 900)],
    groups: [{ label: "Busy", members: Array.from({ length: 16 }, (_, i) => `w${i}`) }, finished] });
  const drawn = budgeted(busy, { w: 900, h: 420 });
  assert.deepEqual(ids(drawn.model, "group"), ["group:Busy"]);
  assert.ok(!ids(drawn.model, "task").includes("task:closed-long-ago"));
  // And what a group counts is what it holds in hand, never the depth of the ledger behind it.
  assert.equal(drawn.hidden.get("group:Busy"), 16 - ids(drawn.model, "task").length);
  assert.equal(drawn.hidden.get("group:Inner"), undefined, "history is not counted");
});

test("a group taken as the centre draws everything it holds in hand, at any depth", () => {
  const model = twoGroups(8), overview = budgeted(model, LAPTOP);
  assert.ok(overview.hidden.get("group:A") > 0, "the overview puts some of A away");
  const inside = budgeted(focusOn(model, "group:A"), LAPTOP);
  assert.equal(ids(inside.model, "task").length, 8);
  assert.equal(inside.hidden.size, 0);
  // Not only the cards hanging off the centre: an inner group's are the branch's too, and they
  // used to have to win room against the window like any other.
  const nested = project({ tasks: Array.from({ length: 12 }, (_, i) => task(`t${i}`, "doing", 1 + i)),
    groups: [{ label: "A", members: ["t0", "t1", "t2", "t3", "t4", "t5"],
      groups: [{ label: "Inner", members: ["t6", "t7", "t8", "t9", "t10", "t11"] }] }] });
  const branch = budgeted(focusOn(nested, "group:A"), LAPTOP);
  assert.equal(ids(branch.model, "task").length, 12);
  assert.equal(branch.hidden.size, 0);
  // It is the one thing here that may overflow, which is what the overview scale and a scroll
  // are for: the window opens at 0.8 rather than shrinking to whatever the day forces.
  assert.ok(arrange(branch.model).height > LAPTOP.h / 0.8);
  assert.equal(opening(arrange(branch.model), LAPTOP), 0.8);
});

test("the tier is what keeps history from crowding the work in hand", () => {
  const model = project({ tasks: [task("stale-open", "todo", 400),
    { ...task("just-done", "done", 0.2), completedAt: hoursAgo(0.2) },
    { ...task("long-done", "done", 300), completedAt: hoursAgo(300) }] });
  const at = (id) => model.nodes.find((n) => n.id === id);
  assert.ok(heat(at("task:stale-open"))[0] > heat(at("task:just-done"))[0],
    "a week-old to-do is in hand; an hour-old closure is a notice");
  assert.ok(heat(at("task:just-done"))[0] > heat(at("task:long-done"))[0],
    "and a notice comes before history");
  // Which is the whole reason the tier exists: on time alone, the freshest thing on a busy
  // instance is almost always something that just finished.
  assert.ok(heat(at("task:just-done"))[1] > heat(at("task:stale-open"))[1]);
});

test("history fills the room a branch's live work leaves, and no more", () => {
  const groups = [{ label: "Shoes", members: ["compare", "research", "pick"],
    groups: [{ label: "Fit", members: ["insoles"] }] }];
  const model = project({ groups, messages: [{ id: "m", role: "user", text: "ok", ts: hoursAgo(0.5) }],
    tasks: [task("compare", "doing", 1), task("insoles", "doing", 2),
      { ...task("research", "done", 300), completedAt: hoursAgo(300) },
      { ...task("pick", "done", 400), completedAt: hoursAgo(400) }] });
  assert.deepEqual(handed(model), ["compare", "insoles"], "the finished pair is history");
  // Pressed in, with the window to itself: both hands, at both depths, and the history after.
  assert.deepEqual(ids(budgeted(focusOn(model, "group:Shoes"), LAPTOP).model, "task").sort(),
    ["task:compare", "task:insoles", "task:pick", "task:research"]);
  // In a window with room for the hands alone, the history is what gives way — and is not
  // counted, because *2 more* is an invitation to press in and history is not one.
  const tight = budgeted(focusOn(model, "group:Shoes"), { w: 760, h: 360 });
  assert.deepEqual(ids(tight.model, "task").sort(), ["task:compare", "task:insoles"]);
  assert.equal(tight.hidden.size, 0);
});

test("a ledger of hundreds costs the window's work, not the ledger's", () => {
  // Every row is a node now, and every offer lays the whole chart out again. What bounds the
  // loop is the candidate list, not the record: everything in hand, then history CANDIDATES deep.
  const model = project({ tasks: [task("open", "doing", 1),
    ...Array.from({ length: 400 }, (_, i) => ({ ...task(`old${i}`, "done", 200 + i), completedAt: hoursAgo(200 + i) }))] });
  assert.equal(ofKind(model, "task").length, 401, "the model is the whole ledger");
  assert.equal(handCount(model), 1);
  const started = Date.now();
  const cut = budgeted(model, LAPTOP);
  assert.ok(Date.now() - started < 2000, "the fitting loop does not grow with the ledger");
  assert.ok(ids(cut.model, "task").includes("task:open"), "the work in hand is drawn first");
  assert.ok(ids(cut.model, "task").length <= 65);
});

test("finished nodes gradually lose emphasis without making their text invisible", () => {
  const fresh = ofKind(project({ tasks: [task("a", "done", 0)] }), "task")[0];
  const old = ofKind(project({ tasks: [task("a", "done", 23)] }), "task")[0];
  assert.equal(emphasis(fresh, NOW), 1);
  assert.ok(emphasis(old, NOW) < 1 && emphasis(old, NOW) >= 0.78);
});
