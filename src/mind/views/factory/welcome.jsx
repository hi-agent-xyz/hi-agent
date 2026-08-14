// purpose: the first hello — a poster shown once when a brand-new person first reaches out.
// A poster, not a page. The seed flags the first meeting; see identity/mod.rs +
// reaction.md. It carries the *feeling* — the real "hi" mark, one line of copy, a warm
// matte ground — while the agent's own voice carries the four ideas (you just talk & work
// with it, it remembers you, it uses your tools, and — the point — it can be taught). No
// wall of text, no tour. Seeded at `factory/welcome`; the agent may adapt it like any view.
import { motion } from "motion/react";
import { url } from "@hi/core";

const CORAL = "#fd605e";
const INK = "#3a352c"; // the house ink, pinned — see `tagline` below
const EASE = [0.22, 0.7, 0.2, 1];

// The real, sealed mark (red h + blue i, white die-cut, soft shadow), seeded beside this
// file and served from the views tree — never re-typed in a system font, never hotlinked.
// Through `url()` because the path is root-absolute: served under the community's subpath
// this poster is at `/ana`, and a bare `/views/…` would ask the community for it instead.
const MARK = url("/views/factory/hi-mark.svg");

// ── words ─────────────────────────────────────────────────────────────────────
// English is the default and the fallback. The tagline is three separate segments on
// purpose — the middle one is painted CORAL — so each one is its own key and the JSX
// composes them; a single sentence per language would lose the coloured beat.
//
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped
// here. Until that exists, an unsupported language lands on English.
const T = {
  en: { talk: "talk to me.", teach: "teach me.", remember: "I remember." },
  zh: { talk: "跟我说话。", teach: "教我。", remember: "我记得。" },
};

// App setting first — the host puts it on `<html lang>` — then the system locale when
// that setting says to follow the person, then English.
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

export default function Welcome() {
  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.985 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.7, ease: EASE }}
      style={S.poster}
    >
      <motion.img
        src={MARK}
        alt="hi"
        draggable={false}
        initial={{ opacity: 0, y: 14 }}
        animate={{ opacity: 1, y: [14, 0, -6, 0] }}
        transition={{
          opacity: { duration: 0.6, ease: EASE, delay: 0.15 },
          y: { duration: 6.5, ease: "easeInOut", times: [0, 0.18, 0.6, 1], repeat: Infinity, delay: 0.15 },
        }}
        style={S.mark}
      />

      <motion.div
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.6, ease: EASE, delay: 0.42 }}
        style={S.tagline}
      >
        {L.talk}&nbsp; <span style={{ color: CORAL }}>{L.teach}</span>&nbsp; {L.remember}
      </motion.div>
    </motion.div>
  );
}

const S = {
  // Pinned to the layer's padding box, not flowed inside it: this poster brings its
  // own fixed ground, and a fixed ground has to cover the frame — left in the flow it
  // would paint only the padded content box and sit in a frame of the host's paper.
  // Safe to cover: the strip is background-only, and the copy is centred well clear
  // of it. A themed view does the opposite and simply stands on the paper.
  poster: {
    position: "absolute",
    inset: 0,
    overflow: "hidden",
    boxSizing: "border-box",
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    justifyContent: "center",
    gap: 30,
    padding: "64px 32px 128px",
    background: "#fff8f3",
    fontFamily: "var(--font-display)",
  },
  mark: {
    position: "relative",
    height: 176,
    width: "auto",
    userSelect: "none",
  },
  tagline: {
    position: "relative",
    fontSize: 21,
    fontWeight: 600,
    letterSpacing: 0,
    // Fixed ink, not `var(--fg)`. This poster paints its own fixed warm ground, so a
    // token that flips with the theme lands cream-on-cream in dark mode — the rule is
    // that a fixed ground takes fixed ink. (Same reason CORAL above is literal: it is
    // the mark's red, not a UI accent.)
    color: INK,
    textAlign: "center",
  },
};
