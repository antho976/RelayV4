#!/usr/bin/env python3
"""Compare two baseline runs: what got slower, what got faster, what changed shape.

    python3 scripts/perf/compare.py docs/perf/runs/2026-09-19 docs/perf/runs/2026-10-03 [--threshold 0.2]

Instructions (callgrind) are the regression signal to trust: they do not move with machine load.
Wall time is reported beside them for the paths that wait on the kernel or a child.
"""
import argparse
import json
from pathlib import Path


def read_jsonl(path):
    if not path.exists():
        return {}
    return {json.loads(l)["name"]: json.loads(l) for l in path.read_text().splitlines() if l.strip()}


def read_json(path):
    return json.loads(path.read_text()) if path.exists() else {}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("before", type=Path)
    ap.add_argument("after", type=Path)
    ap.add_argument("--threshold", type=float, default=0.2, help="relative change worth printing")
    args = ap.parse_args()
    a_native, b_native = read_jsonl(args.before / "native.jsonl"), read_jsonl(args.after / "native.jsonl")
    a_prof, b_prof = read_json(args.before / "profiles.json"), read_json(args.after / "profiles.json")

    rows = []
    for name in sorted(set(a_native) | set(b_native)):
        a, b = a_native.get(name), b_native.get(name)
        if not a or not b:
            rows.append((name, "only in " + ("before" if a else "after"), None, None, None))
            continue
        wall = b["wall_ns"]["p50"] / max(a["wall_ns"]["p50"], 1)
        allocs = b["allocs_per_iter"] / max(a["allocs_per_iter"], 1)
        ir = None
        if name in a_prof and name in b_prof:
            ir = b_prof[name]["ir_per_iter"] / max(a_prof[name]["ir_per_iter"], 1)
        outcome = "" if a.get("codes") == b.get("codes") and a["ok"] == b["ok"] else f"outcome changed: {a.get('codes')} → {b.get('codes')}"
        rows.append((name, outcome, wall, allocs, ir))

    def flag(x):
        if x is None:
            return "–"
        mark = "  ▲" if x > 1 + args.threshold else ("  ▼" if x < 1 - args.threshold else "")
        return f"{x:.2f}×{mark}"

    print(f"{'scenario':42} {'wall p50':>12} {'allocs':>12} {'instructions':>14}  note")
    for name, note, wall, allocs, ir in rows:
        interesting = note or any(x is not None and abs(x - 1) > args.threshold for x in (wall, allocs, ir))
        if interesting:
            print(f"{name:42} {flag(wall):>12} {flag(allocs):>12} {flag(ir):>14}  {note}")
    print()
    print("▲ slower / more, ▼ faster / fewer, beyond the threshold. Rows inside the threshold are not shown.")


if __name__ == "__main__":
    main()
