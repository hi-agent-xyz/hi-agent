import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

// Factory views import host-provided modules. Exercise their layout without the host.
const require = createRequire(new URL("../../../appearance/web/package.json", import.meta.url));
const { flextree } = require("d3-flextree");
const source = readFileSync(new URL("./home.jsx", import.meta.url), "utf8");
const layout = source.slice(0, source.indexOf("export default function Home"))
  .replace(/^import .*;$/gm, "");
const { TONE, HEAT, rowsOf, arrange, wire } = runInNewContext(
  `${layout}\n;({ TONE, HEAT, rowsOf, arrange, wire });`,
  { flextree, document: { documentElement: { lang: "en" } }, navigator: { language: "en" } },
);

test("every task state has a visible wire color, including done", () => {
  for (const tone of HEAT) assert.ok(TONE[tone], `Missing color for ${tone}`);
});

const trunk = {
  key: "__loose", label: "no project", tone: "todo", children: [],
  leaves: [
    { id: "done", title: "Finished view", tone: "done", view: "view" },
    { id: "first", title: "First parked task", tone: "todo" },
    { id: "second", title: "Second parked task", tone: "todo" },
    { id: "__closed", title: "Closed summary", kind: "note", tone: "closed" },
  ],
};

test("unfiled cards, parked tasks and summaries each have a row", () => {
  const rows = rowsOf(trunk);
  assert.equal(rows.length, trunk.leaves.length);
  assert.deepEqual(Array.from(rows, (row) => row.id), trunk.leaves.map((leaf) => leaf.id));
  for (const row of rows.filter((row) => row.t === "chips")) {
    assert.equal(row.chips.length, 1);
    assert.equal(row.tone, row.chips[0].tone);
  }
});

test("every visible item has a distinct finite path from the hub", () => {
  for (const [width, height] of [[960, 520], [2048, 1159]]) {
    const chart = arrange([trunk], width, height);
    const paths = chart.rows.map((row) => wire(chart.hub, row));
    assert.equal(new Set(paths).size, trunk.leaves.length);
    for (const path of paths) {
      assert.match(path, /^M [-\d.]+ [-\d.]+ C /);
      assert.doesNotMatch(path, /NaN|undefined|Infinity/);
    }
  }
});
