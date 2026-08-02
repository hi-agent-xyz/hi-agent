// Built-in: the first-hello — a poster, not a page. Shown once, when a brand-new
// person first reaches out (the seed flags the first meeting; see identity/mod.rs +
// reaction.md). It carries the *feeling* — the real "hi" mark, one line of copy, a warm
// matte ground — while the agent's own voice carries the four ideas (you just talk & work
// with it, it remembers you, it uses your tools, and — the point — it can be taught). No
// wall of text, no tour. Seeded at `_builtin/welcome`; the agent may adapt it like any view.
import { motion } from "motion/react";

const CORAL = "#fd605e";
const EASE = [0.22, 0.7, 0.2, 1];

// The real, sealed mark (red h + blue i, white die-cut, soft shadow), seeded beside this
// file and served from the views tree — never re-typed in a system font, never hotlinked.
const MARK = "/views/_builtin/hi-mark.svg";

export default function Welcome() {
  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.985 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.7, ease: EASE }}
      style={S.poster}
    >
      {/* soft matte colour blooms — the presence, felt behind the mark */}
      <div style={{ ...S.bloom, ...S.bloomCoral }} aria-hidden />
      <div style={{ ...S.bloom, ...S.bloomBlue }} aria-hidden />

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
        talk to me.&nbsp; <span style={{ color: CORAL }}>teach me.</span>&nbsp; I remember.
      </motion.div>
    </motion.div>
  );
}

const S = {
  poster: {
    position: "relative",
    overflow: "hidden",
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    justifyContent: "center",
    gap: 30,
    width: "100%",
    minHeight: 460,
    padding: "64px 48px 56px",
    borderRadius: 30,
    // warm cream → soft sky, matte
    background:
      "linear-gradient(158deg, #fff8f3 0%, #fdf3f4 46%, #eef6ff 100%)",
    border: "1px solid rgba(255,255,255,0.7)",
    boxShadow: "0 30px 80px -30px rgba(24,32,56,0.28), inset 0 1px 0 rgba(255,255,255,0.6)",
    fontFamily: "var(--font-display)",
  },
  bloom: {
    position: "absolute",
    borderRadius: "50%",
    filter: "blur(64px)",
    pointerEvents: "none",
  },
  bloomCoral: {
    width: 340,
    height: 340,
    top: -90,
    left: -70,
    background: "rgba(253,96,94,0.28)",
  },
  bloomBlue: {
    width: 380,
    height: 380,
    bottom: -120,
    right: -90,
    background: "rgba(19,167,245,0.24)",
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
    letterSpacing: "-0.01em",
    color: "var(--fg)",
    textAlign: "center",
  },
};
