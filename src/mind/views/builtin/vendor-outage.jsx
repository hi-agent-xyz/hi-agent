// Built-in: the model is unreachable. Shown by the HOST, not by the agent — which is the
// whole reason it is a bundled view rather than a sentence the agent writes. When the
// upstream vendor is down there is no generation available to phrase anything, so the
// copy has to already exist. `docs/arch/core.md` puts this in the host on purpose: one
// apology per transition, process-wide, never one per scene per retry.
//
// It says two things and stops: your messages are being kept, and you do not have to do
// anything. An outage the person can't act on doesn't want a troubleshooting panel.
//
// LOCALIZATION — English and Chinese now both ship in this file, picked off the same
// language setting the rest of the app follows (`config::KEY_LANGUAGE`, what
// `reaction.md` follows), surfaced to the view on `<html lang>`; English stays the
// default and the fallback. Both variants are written down here for the same reason the
// view is bundled at all: during an outage there is no generation available, so every
// word the person might read has to already exist before the model goes away. That is
// also the limit — a language nobody hand-wrote here lands on English. Further languages
// are the runtime-authored step: the agent reads this surface and writes the variant
// while the model *is* reachable, so the words are on disk before they are needed.
import { motion } from "motion/react";

const EASE = [0.22, 0.7, 0.2, 1];

// ── words ─────────────────────────────────────────────────────────────────────
// TODO(i18n): en + zh are hand-written. Further languages are meant to be authored at
// runtime — the agent reads the surface and writes the variant — rather than shipped
// here. Until that exists, an unsupported language lands on English.
const T = {
  en: {
    title: "I can't reach my model right now",
    body: "I'm keeping everything you send. Nothing is lost — I'll pick it all up together the moment I'm back.",
  },
  zh: {
    title: "我现在连不上我的模型",
    body: "你发的我都留着，一条都不会丢 —— 等我回来一起看。",
  },
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

export default function VendorOutage() {
  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.99 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.5, ease: EASE }}
      style={S.card}
    >
      <div style={{ ...S.bloom }} aria-hidden />

      {/* A slow breath rather than a spinner: this is a wait, not a task in progress. */}
      <motion.div
        animate={{ opacity: [0.35, 0.85, 0.35] }}
        transition={{ duration: 3.2, ease: "easeInOut", repeat: Infinity }}
        style={S.dot}
        aria-hidden
      />

      <motion.h1
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.6, ease: EASE, delay: 0.12 }}
        style={S.title}
      >
        {L.title}
      </motion.h1>

      <motion.p
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.6, ease: EASE, delay: 0.22 }}
        style={S.body}
      >
        {L.body}
      </motion.p>
    </motion.div>
  );
}

const S = {
  card: {
    position: "relative",
    overflow: "hidden",
    width: "100%",
    height: "100%",
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    justifyContent: "center",
    gap: "14px",
    padding: "48px 56px",
    boxSizing: "border-box",
    // No ground of its own: the host already paints the themed view surface behind
    // us, so painting over it is how this used to end up dark-on-dark in light mode.
    color: "var(--fg)",
    textAlign: "center",
  },
  // Matte, recessive, no shine — the ambient register, not an alert banner.
  bloom: {
    position: "absolute",
    inset: "-30%",
    background:
      "radial-gradient(42% 42% at 50% 40%, var(--accent-soft), transparent 70%)",
    filter: "blur(30px)",
    pointerEvents: "none",
  },
  dot: {
    width: "10px",
    height: "10px",
    borderRadius: "50%",
    background: "var(--accent)",
  },
  title: {
    position: "relative",
    margin: 0,
    fontSize: "clamp(20px, 3.4vw, 30px)",
    fontWeight: 620,
    letterSpacing: "-0.015em",
    lineHeight: 1.25,
  },
  body: {
    position: "relative",
    margin: 0,
    maxWidth: "42ch",
    fontSize: "clamp(14px, 1.8vw, 17px)",
    lineHeight: 1.55,
    opacity: 0.72,
  },
};
