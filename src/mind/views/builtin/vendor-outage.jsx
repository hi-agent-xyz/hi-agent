// purpose: the host's own notice that managed calls are paused on a 402. Shown and taken down by the vendor gate, not by the agent.
// Host-owned condition state, not model output. The process-wide vendor gate shows and hides it directly
// from the durable energy level and polls recovery while it is visible.
import { useEffect, useState } from "react";
import { motion } from "motion/react";

const FALLBACK_URL = "https://hi-agent.xyz/account";
const EASE = [0.22, 0.7, 0.2, 1];

const T = {
  en: {
    eyebrow: "ENERGY PAUSED",
    title: "Your energy is used up",
    body: "Your messages are saved. I’ll continue with them as soon as energy returns.",
    action: "Subscribe on hi-agent.xyz",
    note: "A subscription restores access without losing this conversation.",
  },
  zh: {
    eyebrow: "能量已暂停",
    title: "能量已经用完了",
    body: "消息都已保留。能量恢复后，我会继续处理这些消息。",
    action: "前往 hi-agent.xyz 订阅",
    note: "订阅后即可恢复使用，当前对话不会丢失。",
  },
};

function words() {
  const app = document.documentElement.lang || "";
  const chain = !app || /^system$/i.test(app)
    ? [navigator.language]
    : [app, navigator.language];
  for (const tag of chain) {
    if (/^zh\b/i.test(tag || "")) return T.zh;
    if (/^en\b/i.test(tag || "")) return T.en;
  }
  return T.en;
}
const L = words();

export default function OutOfEnergy() {
  const [href, setHref] = useState(FALLBACK_URL);

  useEffect(() => {
    let alive = true;
    fetch("/api/account/subscribe")
      .then((response) => response.json())
      .then((data) => {
        if (alive && typeof data?.url === "string" && data.url) {
          setHref(data.url);
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  return (
    <motion.main
      className="out-of-energy"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.35, ease: EASE }}
      role="alert"
    >
      <style>{`
        .out-of-energy {
          box-sizing: border-box;
          width: 100%;
          height: 100%;
          min-height: 100%;
          display: grid;
          place-items: center;
          padding: max(32px, var(--hi-safe-top)) max(24px, var(--hi-safe-right)) max(32px, var(--hi-safe-bottom)) max(24px, var(--hi-safe-left));
          color: var(--fg);
          font-family: var(--font-display);
          text-align: center;
        }
        .out-of-energy__content {
          width: min(560px, 100%);
          display: flex;
          flex-direction: column;
          align-items: center;
        }
        .out-of-energy__mark {
          width: 72px;
          height: 72px;
          display: grid;
          place-items: center;
          border: 1px solid var(--accent-line);
          border-radius: 50%;
          color: var(--accent);
          background: var(--surface-strong);
          font-size: 30px;
          font-weight: 700;
          line-height: 1;
        }
        .out-of-energy__eyebrow {
          margin: 24px 0 10px;
          color: var(--accent);
          font-family: var(--font-mono);
          font-size: 12px;
          font-weight: 700;
          letter-spacing: 0;
        }
        .out-of-energy__title {
          margin: 0;
          max-width: 18ch;
          font-size: clamp(28px, 5vh, 44px);
          font-weight: 680;
          line-height: 1.1;
          letter-spacing: 0;
        }
        .out-of-energy__body {
          margin: 18px 0 0;
          max-width: 46ch;
          color: var(--fg-dim);
          font-size: 17px;
          line-height: 1.6;
          letter-spacing: 0;
        }
        .out-of-energy__action {
          min-height: 46px;
          margin-top: 30px;
          display: inline-flex;
          align-items: center;
          justify-content: center;
          box-sizing: border-box;
          padding: 0 22px;
          border: 1px solid #fd605e;
          border-radius: 8px;
          color: #fff;
          background: #fd605e;
          font-size: 15px;
          font-weight: 700;
          letter-spacing: 0;
          text-decoration: none;
          transition: background 160ms ease, border-color 160ms ease, box-shadow 160ms ease;
        }
        .out-of-energy__action:hover {
          border-color: #e74d4b;
          background: #e74d4b;
        }
        .out-of-energy__action:focus-visible {
          outline: none;
          box-shadow: 0 0 0 3px var(--bg-0), 0 0 0 5px #fd605e;
        }
        .out-of-energy__note {
          margin: 14px 0 0;
          max-width: 48ch;
          color: var(--fg-mute);
          font-size: 13px;
          line-height: 1.5;
          letter-spacing: 0;
        }
        @media (max-width: 520px) {
          .out-of-energy {
            place-items: start center;
            padding-top: max(72px, var(--hi-safe-top));
          }
          .out-of-energy__mark {
            width: 60px;
            height: 60px;
            font-size: 26px;
          }
          .out-of-energy__title {
            font-size: 30px;
          }
          .out-of-energy__body {
            font-size: 16px;
          }
          .out-of-energy__action {
            width: 100%;
          }
        }
      `}</style>
      <div className="out-of-energy__content">
        <div className="out-of-energy__mark" aria-hidden>0</div>
        <p className="out-of-energy__eyebrow">{L.eyebrow}</p>
        <h1 className="out-of-energy__title">{L.title}</h1>
        <p className="out-of-energy__body">{L.body}</p>
        <a
          className="out-of-energy__action"
          href={href}
          target="_blank"
          rel="noopener noreferrer"
        >
          {L.action}
        </a>
        <p className="out-of-energy__note">{L.note}</p>
      </div>
    </motion.main>
  );
}
