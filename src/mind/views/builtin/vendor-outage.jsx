// Built-in: the model is unreachable. Shown by the HOST, not by the agent — which is the
// whole reason it is a bundled view rather than a sentence the agent writes. When the
// upstream vendor is down there is no generation available to phrase anything, so the
// copy has to already exist. `docs/arch/core.md` puts this in the host on purpose: one
// apology per transition, process-wide, never one per scene per retry.
//
// It says two things and stops: your messages are being kept, and you do not have to do
// anything. An outage the person can't act on doesn't want a troubleshooting panel.
//
// LOCALIZATION — English only, for now. This replaced a hardcoded Chinese string in
// `reactor/mod.rs`, so English is a deliberate step to a neutral default rather than a
// regression from a localized one. The person's language is already a setting
// (`config::KEY_LANGUAGE`, what `reaction.md` follows), and the next move is a
// per-language variant selected off it — a view per language beside this one, picked by
// the host, since the host still cannot generate words at this moment. Not built yet.
import { motion } from "motion/react";

const EASE = [0.22, 0.7, 0.2, 1];

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
        I can't reach my model right now
      </motion.h1>

      <motion.p
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.6, ease: EASE, delay: 0.22 }}
        style={S.body}
      >
        I'm keeping everything you send. Nothing is lost — I'll pick it all up together
        the moment I'm back.
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
    background: "var(--bg, #12151c)",
    color: "var(--fg, #e6ebf5)",
    borderRadius: "18px",
    textAlign: "center",
  },
  // Matte, recessive, no shine — the ambient register, not an alert banner.
  bloom: {
    position: "absolute",
    inset: "-30%",
    background:
      "radial-gradient(42% 42% at 50% 40%, rgba(253,96,94,0.16), rgba(253,96,94,0) 70%)",
    filter: "blur(30px)",
    pointerEvents: "none",
  },
  dot: {
    width: "10px",
    height: "10px",
    borderRadius: "50%",
    background: "#fd605e",
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
