import { describe, it, expect } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

// Two vocabularies, one `:root`.
//
// An agent-written view is a component in a slot of this same page — one React
// tree, no shadow root — so a `<style>` it renders applies to the whole document,
// including this face's chrome. A view bringing its own fixed palette is legal and
// documented (view-builder.md: "Choosing a *fixed* palette is fine and often right
// for a poster"), and it writes the obvious thing:
//
//     :root { --accent: #8a5a2b; --line: #e4ded2; }
//
// Which is how a commentary view repainted the conversation's message bubbles, live,
// on 2026-09-16. The fix is a namespace, not a wall: the chrome reads `--host-*`,
// views read the undecorated names, and `global.css` declares the second as `var()`
// references onto the first. That only holds while nothing in the chrome reads an
// undecorated name — which is what this test is for, because the failure is silent
// and arrives months later, in someone else's view.
//
// Scope is this face's own vocabulary: `global.css` + `tailwind.css` + the chrome's
// own components. Not `ui/shadcn/*` (vendored, and a `shadcn add --overwrite` would
// restore whatever we changed) and not `inspect.css` (/inspect is its own page and
// never shares a document with a view).

const UI = fileURLToPath(new URL(".", import.meta.url));
const ROOT = fileURLToPath(new URL("../../../../../", import.meta.url));

const raw = readFileSync(join(UI, "global.css"), "utf8");
/** Comments stripped: this stylesheet explains itself at length, and the explanations
 *  name tokens in prose. */
const strip = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, "");
const GLOBAL = strip(raw);
const TAILWIND = strip(readFileSync(join(UI, "tailwind.css"), "utf8"));

/** The alias block: every `--x: var(--host-x)` declaration, and nothing else. */
const ALIASES = new Map(
  [...GLOBAL.matchAll(/^\s*(--[a-z0-9-]+):\s*var\((--host-[a-z0-9-]+)\);/gm)].map((m) => [
    m[1]!,
    m[2]!,
  ]),
);

/** `global.css` with the alias block removed — the chrome's own half of the file. */
const CHROME_CSS = GLOBAL.replace(/^\s*--[a-z0-9-]+:\s*var\(--host-[a-z0-9-]+\);$/gm, "");

const uses = (css: string) => [...strip(css).matchAll(/var\(\s*(--[a-z0-9-]+)/g)].map((m) => m[1]!);

/** Every source file the face itself is built from. `inspect/` is its own page (see
 *  above) and `ui/shadcn/` is vendored — a `shadcn add --overwrite` restores it, so a
 *  name held there would not stay held. */
function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const path = join(dir, e.name);
    if (e.isDirectory()) return /^(inspect|shadcn|node_modules)$/.test(e.name) ? [] : walk(path);
    return /\.(css|tsx?|jsx?)$/.test(e.name) && !e.name.includes(".test.") ? [path] : [];
  });
}
const declares = (css: string) => [...css.matchAll(/^\s*(--[a-z0-9-]+)\s*:/gm)].map((m) => m[1]!);

/** A name a view may declare for itself: the palette, under its undecorated name. */
const isPublic = (name: string) => ALIASES.has(name);

describe("the palette a view can overwrite", () => {
  it("aliases the public names onto the chrome's own, one apiece", () => {
    expect(ALIASES.size).toBeGreaterThan(20);
    for (const [pub, priv] of ALIASES) {
      expect(priv, `${pub} aliases the same name`).toBe(`--host-${pub.slice(2)}`);
      expect(declares(GLOBAL).filter((n) => n === pub), `${pub} declared once`).toHaveLength(1);
    }
  });

  it("the chrome reads no name a view may declare", () => {
    for (const [file, css] of [
      ["global.css", CHROME_CSS],
      ["tailwind.css", TAILWIND],
    ] as const) {
      const leaked = uses(css).filter(isPublic);
      expect(leaked, `${file} reads only --host-* names`).toEqual([]);
    }
  });

  it("nothing else the page loads reads a name a view may declare", () => {
    // The whole face, not just this directory: an inline `style={{ background:
    // "var(--accent)" }}` in a hook or a channel is the same leak as a stylesheet.
    for (const file of walk(join(UI, ".."))) {
      const leaked = uses(readFileSync(file, "utf8")).filter(isPublic);
      expect(leaked, `${file.slice(file.indexOf("/src/"))} reads only --host-* names`).toEqual([]);
    }
  });

  // The condition notice is the one view that can be on screen *with* an agent's
  // view — the two live in separate server slots (channels/out/view.ts). Everything
  // else in `factory/` occupies the content slot, which an agent's view replaces
  // rather than covers, so only this one has to be immune.
  it("the condition notice reads no name a view may declare", () => {
    const notice = readFileSync(join(ROOT, "src/mind/views/factory/vendor-outage.jsx"), "utf8");
    expect(uses(notice).filter(isPublic)).toEqual([]);
  });
});

describe("the chrome's own names", () => {
  // `--caption-fg` went with the caption pills in 13504ea and left two references in
  // `tailwind.css` behind it. An undefined custom property is not an error: the
  // declaration is invalid at computed-value time, so `color` quietly inherited and
  // the agent's message bubble sat at 1.82:1 on the dark skin. Nothing failed.
  it("declares every --host-* name it reads", () => {
    const declared = new Set(declares(GLOBAL));
    for (const [file, css] of [
      ["global.css", GLOBAL],
      ["tailwind.css", TAILWIND],
    ] as const) {
      for (const name of uses(css).filter((n) => n.startsWith("--host-"))) {
        expect(declared, `${file} reads ${name}, which global.css declares`).toContain(name);
      }
    }
  });

  it("declares the skin-flipping ones in every skin", () => {
    // Three blocks carry a colour: `:root`, the OS-dark media query, and the forced
    // `[data-theme="dark"]` override. A token added to one and forgotten in the other
    // two is a colour that only works in one skin — and the forced-dark block is the
    // one that gets forgotten, because the OS on this desk is usually telling the
    // truth. So: declared as many times as the accent is, whatever that count is.
    const times = (name: string) =>
      [...GLOBAL.matchAll(new RegExp(`^\\s*${name}\\s*:`, "gm"))].length;
    const skins = times("--host-accent");
    expect(skins, "the accent is restated per skin").toBe(3);
    for (const name of ["--host-fg", "--host-fill-ink", "--host-surface", "--host-line"]) {
      expect(times(name), `${name} is declared in every skin`).toBe(skins);
    }
  });
});
