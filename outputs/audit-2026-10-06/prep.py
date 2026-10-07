"""Split the kept round-1 findings into dedup shards and coverage.md into halves."""
import json
import os
import sys

AUD = sys.argv[1]
OUT = sys.argv[2]
d = json.load(open(os.path.join(AUD, "round1.json")))
fs = [f for r in d["results"] for f in r["findings"] if f["status"] != "refuted"]


def shard_of(p):
    if p.startswith("crates/relay-core/tests/") or p.startswith("crates/relay-core/examples/"):
        return "tests-perf"
    if p.startswith("crates/relay-core/src/handlers/"):
        name = p.rsplit("/", 1)[1].removesuffix(".rs")
        if name in ("session", "task", "notes", "module", "overlap", "guardrail", "audit", "bus"):
            return "handlers-core"
        return "handlers-other"
    if p.startswith("crates/relay-core/"):
        return "engine-core"
    if p.startswith("apps/relay-native/"):
        name = p.removeprefix("apps/relay-native/src/").split("/")[0].removesuffix(".rs")
        if name in ("app", "client", "shell", "board", "code_git", "editor", "project_files", "image_preview"):
            return "native-main"
        return "native-other"
    return "bus-cli-remote-misc"


shards = {}
for f in fs:
    shards.setdefault(shard_of(f["file"]), []).append(f)

# Split shards over ~130 findings by file, keeping a file's findings together.
final = {}
for name, items in shards.items():
    items.sort(key=lambda f: (f["file"], f.get("line") or 0))
    if len(items) <= 130:
        final[name] = items
        continue
    parts = -(-len(items) // 120)
    target = -(-len(items) // parts)
    cur, idx = [], 1
    for i, f in enumerate(items):
        cur.append(f)
        nxt = items[i + 1]["file"] if i + 1 < len(items) else None
        if len(cur) >= target and nxt != f["file"]:
            final[f"{name}-{idx}"] = cur
            cur, idx = [], idx + 1
    if cur:
        final[f"{name}-{idx}"] = cur


def compact(f):
    return {
        "id": f["id"], "sev": f["final_severity"], "cat": f["final_category"], "status": f["status"],
        "skeptic": bool(f.get("v2")), "loc": f"{f['file']}:{f.get('line')}", "symbol": f.get("symbol"),
        "title": f["title"], "related": f.get("related") or [], "evidence": f["evidence"][:700],
        "fix": f["fix"][:300],
    }


os.makedirs(OUT, exist_ok=True)
manifest = []
for name, items in sorted(final.items()):
    path = os.path.join(OUT, f"shard-{name}.jsonl")
    with open(path, "w") as fh:
        for f in items:
            fh.write(json.dumps(compact(f)) + "\n")
    files = sorted({f["file"] for f in items})
    manifest.append({"key": name, "path": path, "count": len(items), "files": len(files)})
    print(name, len(items), len(files), os.path.getsize(path))

# Coverage notes, split in two halves by unit.
units = [(r["unit"], r["kind"], r.get("coverage") or "") for r in d["results"]]
half = len(units) // 2
for i, part in enumerate((units[:half], units[half:]), start=1):
    path = os.path.join(OUT, f"coverage-{i}.md")
    with open(path, "w") as fh:
        for u, k, c in part:
            fh.write(f"## {u} ({k})\n\n{c}\n\n")
    manifest.append({"key": f"coverage-{i}", "path": path, "units": len(part)})
    print(path, os.path.getsize(path))
json.dump(manifest, open(os.path.join(OUT, "manifest.json"), "w"), indent=1)
