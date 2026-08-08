#!/usr/bin/env python3
"""One-shot migration: fold the per-scene raw memory into the flat layout.

`memory/raw/<scene>/<channel>/<date>/…` becomes `memory/raw/<channel>/<date>/…`,
because there is one conversation and it has no name (see
`docs/arch/core.md#one-conversation`).

Merging, not picking a winner. Every scene a real install accumulated was the
same person — a browser profile wearing the name of a situation — so keeping one
and dropping the rest would throw away that person's own history. Journal lines
from all scenes are concatenated per `(channel, date)` and sorted by `(ts, id)`,
which is the order `journal::recent` reads them back in anyway. Media blobs move
alongside; a name collision keeps both by suffixing, since the bytes are the
lossless record and losing one is worse than an ugly filename.

`raw/sessions/` is left alone — it is the ACP frame log, not a scene.

Idempotent: a second run finds nothing left under a scene directory and exits.
Safe by default — pass --apply to actually move anything.
"""

import argparse
import json
import shutil
import sys
from pathlib import Path

# Children of raw/ that are not scenes. Keep in step with `layout::SESSIONS_DIR`.
NOT_A_SCENE = {"sessions"}
# Channel/state directories a scene could hold. Everything else under a scene is
# carried over verbatim under the same name.
KNOWN = {"text", "audio", "vision", "file", "view", "clock", "worker", "appearance", "files"}


def scene_dirs(raw: Path) -> list[Path]:
    out = []
    for d in sorted(raw.iterdir()):
        if not d.is_dir() or d.name in NOT_A_SCENE:
            continue
        # A directory that is already a channel is the flat layout, not a scene.
        if d.name in KNOWN:
            continue
        out.append(d)
    return out


def entry_key(line: str):
    try:
        e = json.loads(line)
        return (e.get("ts", ""), e.get("id", ""))
    except Exception:
        # Unparseable lines sort last rather than being dropped: raw is the
        # lossless record, and a line we cannot read is still a line that happened.
        return ("￿", line)


def merge_jsonl(sources: list[Path], dest: Path, apply: bool) -> int:
    lines: list[str] = []
    for s in sources:
        lines.extend(x for x in s.read_text(encoding="utf-8").splitlines() if x.strip())
    lines.sort(key=entry_key)
    if apply:
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return len(lines)


def unique(dest: Path) -> Path:
    if not dest.exists():
        return dest
    n = 1
    while True:
        cand = dest.with_name(f"{dest.stem}-{n}{dest.suffix}")
        if not cand.exists():
            return cand
        n += 1


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("data_dir", type=Path)
    ap.add_argument("--apply", action="store_true", help="actually move files")
    args = ap.parse_args()

    raw = args.data_dir / "memory" / "raw"
    if not raw.is_dir():
        print(f"no raw store at {raw}", file=sys.stderr)
        return 1

    scenes = scene_dirs(raw)
    if not scenes:
        print("nothing to migrate — the layout is already flat")
        return 0
    print(f"scenes to fold: {', '.join(d.name for d in scenes)}")

    # (channel, day, logname) -> [source logs];  and a list of loose blobs to move.
    logs: dict[tuple[str, str, str], list[Path]] = {}
    blobs: list[tuple[Path, Path]] = []

    for sc in scenes:
        for chan in sorted(p for p in sc.iterdir() if p.is_dir()):
            for day in sorted(p for p in chan.iterdir() if p.is_dir()):
                for item in sorted(day.rglob("*")):
                    if item.is_dir():
                        continue
                    rel = item.relative_to(day)
                    if item.suffix == ".jsonl" and item.parent == day:
                        logs.setdefault((chan.name, day.name, item.name), []).append(item)
                    else:
                        blobs.append((item, raw / chan.name / day.name / rel))

    total = 0
    for (chan, day, name), sources in sorted(logs.items()):
        dest = raw / chan / day / name
        existing = [dest] if dest.exists() else []
        n = merge_jsonl(existing + sources, dest, args.apply)
        total += n
        print(f"  {chan}/{day}/{name}: {n} entries from {len(sources)} scene(s)")

    moved = 0
    for src, dest in blobs:
        target = unique(dest)
        if args.apply:
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(src), str(target))
        moved += 1

    # The carried-forward briefs: concatenate under a divider rather than picking
    # one. Deliberation rewrites this file whole on its next turn, so a merged
    # file self-heals; a lost one does not.
    prompts = args.data_dir / "memory" / "prompts"
    old_scenes = prompts / "scenes"
    if old_scenes.is_dir():
        parts = []
        for f in sorted(old_scenes.glob("*.md")):
            body = f.read_text(encoding="utf-8").strip()
            if body:
                parts.append(body)
        if parts:
            merged = "\n\n---\n\n".join(parts) + "\n"
            print(f"  prompts: merging {len(parts)} brief(s) -> prompts/conversation.md")
            if args.apply:
                (prompts / "conversation.md").write_text(merged, encoding="utf-8")
        if args.apply:
            shutil.rmtree(old_scenes)

    if args.apply:
        for sc in scenes:
            shutil.rmtree(sc)
        print(f"folded {total} journal entries and {moved} blobs; removed {len(scenes)} scene dirs")
    else:
        print(f"\ndry run — would fold {total} journal entries and {moved} blobs")
        print("re-run with --apply to do it")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
