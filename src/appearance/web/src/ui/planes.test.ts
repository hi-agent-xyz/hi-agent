import { describe, it, expect } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

// The stack is static and declared once — three planes, ground / view / cover,
// and the numbers are 0, 1, 2 (docs/arch/stage.md). This test is the thing that
// keeps it that way, because the failure it guards against is silent: a surface
// picks a number that happens to work today, and two years of that is how the
// application arrived at 2 / 2 / 8 / 50 / 55 / 60 / 60 / 60 / 80, with the
// conversation parked underneath the view plane and no one able to say why.

const UI = fileURLToPath(new URL(".", import.meta.url));
// Comments stripped first: this stylesheet explains itself at length, and several
// of those explanations name `z-index:` in prose.
const CSS = readFileSync(join(UI, "global.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

const PLANES = ["--z-ground", "--z-view", "--z-cover"];

/** Every `z-index: <value>` in the stylesheet, in source order. */
function declarations(css: string): string[] {
  return [...css.matchAll(/z-index:\s*([^;]+);/g)].map((m) => m[1]!.trim());
}

describe("the three planes", () => {
  it("declares exactly three plane tokens, and they are 0, 1, 2", () => {
    for (const [i, token] of PLANES.entries()) {
      const declared = [...CSS.matchAll(new RegExp(`${token}:\\s*([^;]+);`, "g"))];
      expect(declared, `${token} is declared once`).toHaveLength(1);
      expect(declared[0]![1]!.trim(), `${token} is ${i}`).toBe(String(i));
    }
  });

  it("only the plane wrappers read the tokens, one apiece", () => {
    const used = declarations(CSS).filter((v) => v.startsWith("var("));
    expect(used.sort()).toEqual(PLANES.map((p) => `var(${p})`).sort());
  });

  // A contained stacking context never needs 50. A larger value is evidence that
  // someone was fighting a stack they could not see — which is exactly the state
  // this replaced, and exactly what would quietly reintroduce it.
  it("inside a plane, z-index is a single digit", () => {
    const literals = declarations(CSS).filter((v) => !v.startsWith("var("));
    for (const value of literals) {
      expect(Number(value), `${value} is a single digit`).toBeLessThan(10);
    }
  });

  // The one that got away last time: `ViewSlot` set `zIndex: 50` in a style prop,
  // where no stylesheet reader would ever find it.
  it("no component sets z-index in a style prop", () => {
    const offenders: string[] = [];
    const walk = (dir: string) => {
      for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const path = join(dir, entry.name);
        if (entry.isDirectory()) {
          walk(path);
        } else if (/\.tsx?$/.test(entry.name) && !entry.name.endsWith(".test.ts")) {
          if (/zIndex/.test(readFileSync(path, "utf8"))) offenders.push(path);
        }
      }
    };
    walk(join(UI, ".."));
    expect(offenders).toEqual([]);
  });
});
