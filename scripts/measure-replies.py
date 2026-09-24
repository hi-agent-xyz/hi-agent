#!/usr/bin/env python3
"""How fast Reaction answers, and what it says — measured off the wire log.

Everything here is read from `data/memory/raw/`, which the host writes as it runs:
the text channel's own messages, and every JSON-RPC frame in both directions for
each agent session. Nothing is inferred from the conversation itself, which is the
point — a journey that asks "did it answer in one turn" cannot be settled by
reading the reply.

    python3 scripts/measure-replies.py [DATA_DIR]

DATA_DIR defaults to `data`. See docs/user-journeys/measuring.md for what each
number means, where its boundaries are, and the baseline to compare against.
"""

import json
import glob
import os
import re
import sys
from datetime import datetime, timezone

# --------------------------------------------------------------------------
# Reading the corpus
# --------------------------------------------------------------------------


def ts(s):
    """RFC3339 as the host writes it → epoch seconds."""
    return None if s is None else datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


def text_messages(root):
    """(when, who, what) for the text channel. `who` is 'them' or 'agent'."""
    out = []
    for path in sorted(glob.glob(f"{root}/text/*/text.jsonl")):
        for line in open(path):
            try:
                o = json.loads(line)
            except ValueError:
                continue
            if o.get("kind") != "message":
                continue
            m = o.get("message") or {}
            at = ts(m.get("ts"))
            if at:
                who = "agent" if m.get("from") == "agent" else "them"
                out.append((at, who, (m.get("content") or {}).get("text") or ""))
    out.sort()
    return out


def turns(root, role):
    """One dict per turn of `role`, in order.

    `start` is when the host sent `turn/start`; `says` are the lines it *wrote* —
    the `finished` says of each `hi_prepare` the host took, or, in a log from before
    `hi_prepare` was the way out, the `hi_say` calls; `end` is `turn/completed`.
    `requests` counts the `tokenUsage` updates seen before the first line, which is how
    a turn that took one upstream request is told from one that took several.

    A line written is not a line said: the floor lets it out when the room reaches its
    moment, and what reached the conversation, and when, is `text_messages`'.
    """
    out = []
    for path in sorted(glob.glob(f"{root}/sessions/*/{role}.jsonl")):
        cur = None
        for line in open(path):
            try:
                o = json.loads(line)
            except ValueError:
                continue
            at, method = ts(o.get("ts")), o.get("method")
            if method == "turn/start" and o.get("dir") == "send":
                raw = json.loads(o["raw"])
                prompt = "".join(i.get("text") or "" for i in raw["params"].get("input", []))
                cur = {"start": at, "prompt": prompt, "says": [], "end": None,
                       "requests": 0, "usage": None}
                out.append(cur)
            elif cur is None:
                continue
            elif method == "item/completed":
                try:
                    item = json.loads(o["raw"])["params"].get("item", {})
                except (ValueError, KeyError):
                    continue
                args = item.get("arguments") or {}
                if item.get("tool") == "hi_say":
                    cur["says"].append({"at": at, "text": args.get("text") or ""})
                elif item.get("tool") == "hi_prepare" and prepared(item):
                    for b in args.get("branches") or []:
                        if b.get("when") != "finished":
                            continue
                        for a in b.get("actions") or []:
                            if a.get("do") == "say":
                                cur["says"].append({"at": at, "text": a.get("text") or ""})
            elif method == "thread/tokenUsage/updated":
                if not cur["says"]:
                    cur["requests"] += 1
                elif cur["usage"] is None:
                    try:
                        cur["usage"] = json.loads(o["raw"])["params"]["tokenUsage"].get("last")
                    except (ValueError, KeyError):
                        pass
            elif method == "turn/completed":
                cur["end"] = at
    out.sort(key=lambda t: t["start"])
    return out


def prepared(item):
    """Whether the host took a `hi_prepare` call — its answer starts `prepared`."""
    result = item.get("result") or {}
    text = "".join(c.get("text") or "" for c in result.get("content") or [])
    return text.startswith("prepared")


def first_reply(msgs, at):
    """When the agent's first message after `at` reached the conversation."""
    for t, who, _ in msgs:
        if t > at and who == "agent":
            return t
    return None


def signals(prompt):
    """The `>` lines of a prompt's `## New signals` block — what a person said."""
    if "## New signals" not in prompt:
        return []
    block = prompt[prompt.index("## New signals"):]
    return [l[1:].strip() for l in block.splitlines() if l.startswith(">")]


def key(s):
    return re.sub(r"\s+", "", s)[:60]


def pair(msgs, rxn):
    """Each thing a person said, with the Reaction turn that first carried it.

    Matched on the message's own text appearing in the turn's `## New signals`,
    never on time alone: a turn can open seconds or minutes after they spoke, and
    the settle window means several utterances share one turn.
    """
    carried = {}
    for t in rxn:
        for line in signals(t["prompt"]):
            carried.setdefault(key(line), []).append(t)
    out = []
    for at, who, text in msgs:
        if who != "them" or not key(text):
            continue
        cands = [t for t in carried.get(key(text), []) if t["start"] >= at - 2]
        if cands:
            out.append({"said": at, "text": text, "turn": min(cands, key=lambda t: t["start"])})
    return out


# --------------------------------------------------------------------------
# Reporting
# --------------------------------------------------------------------------


def branch_events(root):
    """The prepared-branch events from the observatory's own log, `<data>/sessions.jsonl`.

    Read there rather than from the wire: what a reading decided — and how long after
    their message the first action ran — is the host's, not any session's."""
    path = os.path.join(root, "sessions.jsonl")
    if not os.path.exists(path):
        return []
    out = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            try:
                e = json.loads(line)
            except ValueError:
                continue
            if str(e.get("event", "")).startswith(("branches_", "floor_")):
                out.append(e)
    return out


def q(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(p * (len(xs) - 1))))]


def row(name, xs, unit="s"):
    if not xs:
        print(f"  {name:<44s} n=0")
        return
    print(f"  {name:<44s} n={len(xs):>5d}  med={q(xs, .5):>7.1f}{unit}"
          f"  p75={q(xs, .75):>7.1f}  p90={q(xs, .9):>7.1f}  max={max(xs):>8.1f}")


def pct(name, hit, total):
    share = 100 * hit / total if total else 0.0
    print(f"  {name:<44s} {hit:>5d} / {total:<5d} ({share:4.1f}%)")


def main(root):
    raw = os.path.join(root, "memory", "raw")
    if not os.path.isdir(raw):
        sys.exit(f"no wire log at {raw} — point this at a data dir the host has run in")
    msgs, rxn = text_messages(raw), turns(raw, "reaction")
    paired = pair(msgs, rxn)
    spoke = [t for t in rxn if t["says"]]
    done = [t for t in rxn if t["end"]]
    if not paired:
        sys.exit("no person-message could be paired to a Reaction turn; nothing to report")

    span = (datetime.fromtimestamp(min(m[0] for m in msgs), timezone.utc).date(),
            datetime.fromtimestamp(max(m[0] for m in msgs), timezone.utc).date())
    print(f"\n{span[0]} … {span[1]}   "
          f"{sum(1 for m in msgs if m[1] == 'them')} things they said, "
          f"{len(rxn)} Reaction turns, {len(paired)} paired\n")

    # -- Felt latency, split at the turn boundary --------------------------
    print("延迟 · 说完 → 第一句")
    row("A  说完 → turn/start", [p["turn"]["start"] - p["said"] for p in paired])
    row("B  turn/start → 第一句写好",
        [p["turn"]["says"][0]["at"] - p["turn"]["start"] for p in paired if p["turn"]["says"]])
    row("A+B 说完 → 第一句写好",
        [p["turn"]["says"][0]["at"] - p["said"] for p in paired if p["turn"]["says"]])
    # What they felt: the first message of ours in the conversation after theirs — the floor
    # may let a line out after it was written, or a line written before this one of theirs.
    felt = [r - p["said"] for p in paired if (r := first_reply(msgs, p["said"])) is not None]
    row("说完 → 第一句出现在对话里", felt)

    # A is two populations: the settle window, and waiting out a running turn.
    live = sorted((t["start"], t["end"], t) for t in done)

    def running_at(at):
        for s, e, t in live:
            if s <= at <= e:
                return t
            if s > at:
                break
        return None

    blocked, free = [], []
    for p in paired:
        r = running_at(p["said"])
        if r is not None and r is not p["turn"]:
            blocked.append(r["end"] - p["said"])
        else:
            free.append(p["turn"]["start"] - p["said"])
    pct("你开口时它正在跑（对话轮次重叠）", len(blocked), len(paired))
    row("  等那一轮跑完", blocked)
    row("  没在跑时的 A（settle 窗口本身）", free)

    # -- Prepared branches: the reply that skipped the turn ---------------
    events = branch_events(root)
    if events:
        print("\n备好的分支 · hi_prepare")
        sets = [e for e in events if e["event"] == "branches_prepared" and e.get("directions")]
        read = [e for e in events if e["event"] == "branches_resolved"]
        voided = [e for e in events if e["event"] == "branches_voided"]
        # A set now waits for its matter, so one set can be read against many messages:
        # the two counts sit side by side, not as a ratio.
        print(f"  备下的组 {len(sets)} · 有备着的组时他说的话 {len(read)}")
        pct("  其中谈到了备着的事(用掉)", sum(1 for e in read if e.get("used_up")), len(read))
        outcomes = {}
        for e in read:
            outcomes[e.get("outcome", "?")] = outcomes.get(e.get("outcome", "?"), 0) + 1
        for name, n in sorted(outcomes.items(), key=lambda kv: -kv[1]):
            pct(f"  {name}", n, len(read))
        row("他的话 → 分支第一个动作",
            [e["first_action_ms"] / 1000 for e in read if e.get("first_action_ms") is not None])
        row("他的话 → 读出结果", [e["decided_ms"] / 1000 for e in read if "decided_ms" in e])
        reasons = {}
        for e in voided:
            reasons[e.get("reason", "?")] = reasons.get(e.get("reason", "?"), 0) + 1
        for name, n in sorted(reasons.items(), key=lambda kv: -kv[1]):
            pct(f"  作废: {name}", n, len(sets))

    floor = [e for e in events if e["event"] == "floor_read"]
    if floor:
        print("\n地板 · 每次停下的判定")
        outcomes = {}
        for e in floor:
            outcomes[e.get("outcome", "?")] = outcomes.get(e.get("outcome", "?"), 0) + 1
        for name, n in sorted(outcomes.items(), key=lambda kv: -kv[1]):
            pct(f"  {name}", n, len(floor))
        row("停下 → 判定", [e["decided_ms"] / 1000 for e in floor if "decided_ms" in e])
        pct("  放出了东西", sum(1 for e in floor if e.get("released")), len(floor))

    # -- The generation: what one turn costs --------------------------------
    print("\n地板 · 一轮的成本")
    row("turn 全长", [t["end"] - t["start"] for t in done])
    first = [t["says"][0]["at"] - t["start"] for t in spoke]
    row("turn/start → 第一句写好", first)
    row("最后一句之后还在跑", [t["end"] - t["says"][-1]["at"] for t in spoke if t["end"]])
    pct("第一句之前 0 次 tokenUsage（= 一次上游请求）",
        sum(1 for t in spoke if t["requests"] == 0), len(spoke))
    usage = [t["usage"] for t in spoke if t["usage"]]
    if usage:
        row("  那一次请求的输出 token", [float(u.get("outputTokens", 0)) for u in usage], " tok")
        row("  其中 reasoning", [float(u.get("reasoningOutputTokens", 0)) for u in usage], " tok")
        row("  输入 token", [float(u.get("inputTokens", 0)) for u in usage], " tok")
        cached = [100.0 * u.get("cachedInputTokens", 0) / u["inputTokens"]
                  for u in usage if u.get("inputTokens")]
        row("  其中命中缓存", cached, "%")

    # -- What it says ------------------------------------------------------
    print("\n话 · 第一句的形状")
    opening = [p["turn"]["says"][0]["text"] for p in paired if p["turn"]["says"]]
    row("每句字数", [float(len(s["text"])) for t in rxn for s in t["says"]], "字")
    row("每轮说几句", [float(len(t["says"])) for t in spoke], "句")
    patterns = {
        "开头先复述（明白/好的/收到…）": r"^\s*(明白|好的|收到|了解|懂了|OK|ok)[，,。！!、\s]",
        "报时间估计（给我X分钟）":
            r"(约|大约|预计|大概|需要|给我)\s*[0-9一两三四五六七八九十几半]+\s*(分钟|秒|小时|min)",
        "「完成后给你一份…报告」":
            r"(完成后|之后|随后|届时|然后)[^。；\n]{0,12}(给你|发你|提供|输出)[^。；\n]{0,14}"
            r"(报告|清单|总结|文档|结果)",
        "「有不清楚的我再问你」":
            r"(有|若|如果)[^。；\n]{0,10}(不明白|不清楚|拿不准|疑问)[^。；\n]{0,10}(再|会)"
            r"[^。；\n]{0,6}(问|找|确认)你",
    }
    for name, rx in patterns.items():
        pat = re.compile(rx)
        pct(name, sum(1 for s in opening if pat.search(s)), len(opening))

    # -- What it was given -------------------------------------------------
    print("\n窗口 · 它被给到了什么")
    for block in ("## New signals", "# Active tasks", "## What I carry forward",
                  "## Still looking into", "## On screen now",
                  "## What your words have earned", "## Recent (last 30 minutes)"):
        pct(block, sum(1 for t in rxn if block in t["prompt"]), len(rxn))
    ledger = [t for t in rxn if "# Active tasks" in t["prompt"]]
    cut = re.compile(r"and (\d+) more active")
    pct("  账本被截断（行掉到计数里）",
        sum(1 for t in ledger if cut.search(t["prompt"])), len(ledger))
    print()


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "data")
