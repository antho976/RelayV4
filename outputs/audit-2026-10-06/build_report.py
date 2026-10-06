"""Assemble the audit report from the workflow results.

Usage: python3 -I build_report.py <config.json>

config.json:
  {
    "rounds": ["round2-merged.json"],           # workflow return values: {"summary":..., "results":[...]}
                                                 # a finding may carry v3 (blind re-check), recheck, dispute;
                                                 # status may be confirmed, plausible, disputed, fixed, refuted
    "dedup": "dedup.json",                       # optional: {"groups":[{"keep":id,"dupes":[ids],"note":str}]}
    "header": "header.md",                       # hand-written front matter (summary, themes, tooling)
    "out_md": ".../outputs/code-audit-2026-10-06.md",
    "out_json": ".../report-data.json"
  }
"""
import json
import sys
from collections import Counter, defaultdict

SEV_ORDER = {"critical": 0, "high": 1, "medium": 2, "low": 3}
DEFECT_CATS = {"bug", "security", "concurrency", "invariant", "performance", "resource-leak", "error-handling", "data-loss"}
QUALITY_CATS = {"dead-code", "duplication", "overengineering", "code-quality", "test-quality", "docs-drift"}
CAT_LABEL = {
    "bug": "Bug", "security": "Security", "concurrency": "Concurrency", "invariant": "Invariant",
    "performance": "Performance", "resource-leak": "Resource leak", "error-handling": "Error handling",
    "data-loss": "Data loss", "dead-code": "Dead code", "duplication": "Duplication",
    "overengineering": "Overengineering", "code-quality": "Code quality", "test-quality": "Test quality",
    "docs-drift": "Docs drift",
}

AREAS = [
    ("crates/relay-core/src/handlers/", "Engine: handlers"),
    ("crates/relay-core/src/", "Engine: core"),
    ("crates/relay-core/tests/", "Engine: tests and perf"),
    ("crates/relay-core/examples/", "Engine: tests and perf"),
    ("crates/relay-core/", "Engine: core"),
    ("crates/relay-bus/", "Bus types"),
    ("crates/relay-cli/src/blender_py/", "CLI: embedded Python"),
    ("crates/relay-cli/src/unreal_py/", "CLI: embedded Python"),
    ("crates/relay-cli/src/remote.rs", "Phone door"),
    ("crates/relay-cli/", "CLI and MCP servers"),
    ("crates/relay-remote/", "Phone door"),
    ("deploy/", "Phone door"),
    ("apps/relay-native/src/css/", "Native client: CSS and resources"),
    ("apps/relay-native/src/theme.css", "Native client: CSS and resources"),
    ("apps/relay-native/resources/", "Native client: CSS and resources"),
    ("apps/relay-native/src/smoke", "Native client: smoke harness"),
    ("apps/relay-native/src/roadmap_smoke", "Native client: smoke harness"),
    ("apps/relay-native/", "Native client"),
    ("scripts/", "Scripts, CI and build"),
    (".github/", "Scripts, CI and build"),
    ("run.sh", "Scripts, CI and build"),
    ("Cargo.toml", "Scripts, CI and build"),
    ("Cargo.lock", "Scripts, CI and build"),
    ("plugins/", "Plugins"),
    ("docs/", "Docs"),
    ("schema/", "Bus types"),
    ("CLAUDE.md", "Docs"),
    ("README.md", "Docs"),
    ("DESIGN.md", "Docs"),
]


def area_of(path):
    for prefix, name in AREAS:
        if path.startswith(prefix):
            return name
    return "Other"


def load_rounds(paths):
    out = []
    for rnd, p in enumerate(paths, start=1):
        data = json.load(open(p))
        for unit in data["results"]:
            if not unit:
                continue
            for f in unit.get("findings", []):
                f = dict(f)
                f["round"] = rnd
                f["unit"] = unit.get("unit")
                f["unit_kind"] = unit.get("kind")
                out.append(f)
    return out


def coverage_notes(paths):
    notes = []
    for p in paths:
        data = json.load(open(p))
        for unit in data["results"]:
            if unit and unit.get("coverage"):
                notes.append((unit.get("unit"), unit.get("coverage")))
    return notes


NOTE_LABEL = {"v1": "Verifier", "v2": "Skeptic", "v3": "Blind re-check", "v_recheck": "Recheck after PR #30/#31"}


def corrections(f):
    """Each verifier's correction, labelled with the pass that made it, oldest first."""
    parts = []
    for key in ("v1", "v2", "v3", "v_recheck"):
        v = f.get(key) or {}
        c = (v.get("correction") or "").strip()
        if c and c.lower() not in ("none", "n/a", "-", "none."):
            parts.append(f"{NOTE_LABEL[key]}: {c}")
    return parts


def md_escape_title(t):
    return t.replace("\n", " ").strip()


def render_finding(f, rid):
    loc = f"{f['file']}:{f['line']}" if f.get("line") else f["file"]
    status = f["status"]
    head = f"<a id=\"{rid.lower()}\"></a>\n\n#### {rid} · {md_escape_title(f['title'])}\n"
    meta = (f"`{loc}` · **{f['final_severity']}** · {CAT_LABEL.get(f['final_category'], f['final_category'])}"
            f" · {status} · effort {f.get('effort', '?')}")
    if f.get("recheck"):
        meta += f" · {f['recheck']}"
    lines = [head, meta + "\n"]
    lines.append(f"**Problem.** {f['evidence'].strip()}\n")
    lines.append(f"**Impact.** {f['impact'].strip()}\n")
    lines.append(f"**Fix.** {f['fix'].strip()}\n")
    for c in corrections(f):
        label, _, text = c.partition(": ")
        lines.append(f"**{label} note.** {text}\n")
    if f.get("dispute"):
        lines.append(f"**Disputed.** {f['dispute'].strip()}\n")
    if status == "plausible":
        v = f.get("v2") or f.get("v1") or {}
        if v.get("reason"):
            lines.append(f"**Why only plausible.** {v['reason'].strip()}\n")
    rel = [r for r in (f.get("related") or []) if r]
    if rel:
        lines.append("**Also at:** " + ", ".join(f"`{r}`" for r in rel) + "\n")
    if f.get("merged_from"):
        lines.append("**Merged duplicates:** " + ", ".join(f.get("merged_from")) + "\n")
    return "\n".join(lines)


def main():
    cfg = json.load(open(sys.argv[1]))
    allf = load_rounds(cfg["rounds"])
    byid = {f["id"]: f for f in allf}

    # Dedup: fold duplicates into the kept finding.
    dropped = set()
    if cfg.get("dedup"):
        groups = json.load(open(cfg["dedup"])).get("groups", [])
        for g in groups:
            keep = byid.get(g["keep"])
            if not keep:
                continue
            for d in g.get("dupes", []):
                if d == g["keep"] or d not in byid:
                    continue
                dup = byid[d]
                dropped.add(d)
                keep.setdefault("merged_from", []).append(d)
                # A duplicate never lowers severity; take the more severe verified rating,
                # unless a blind round-2 re-check (v3) already calibrated the kept finding.
                if (not keep.get("v3") and dup["status"] in ("confirmed", "plausible") and
                        SEV_ORDER[dup["final_severity"]] < SEV_ORDER[keep["final_severity"]]):
                    keep["final_severity"] = dup["final_severity"]
                for r in [f"{dup['file']}:{dup.get('line')}"] + (dup.get("related") or []):
                    if r not in (keep.get("related") or []) and r != f"{keep['file']}:{keep.get('line')}":
                        keep.setdefault("related", []).append(r)

    live = []
    disputed = []
    unverified = []
    fixed = []
    refuted = 0
    for f in allf:
        if f["id"] in dropped:
            continue
        f["area"] = area_of(f["file"])
        s = f["status"]
        if s in ("confirmed", "plausible"):
            live.append(f)
        elif s == "disputed":
            disputed.append(f)
        elif s == "unverified":
            unverified.append(f)
        elif s == "fixed":
            fixed.append(f)
        else:
            refuted += 1

    def sort_key(f):
        return (SEV_ORDER[f["final_severity"]], 0 if f["status"] == "confirmed" else 1, f["area"], f["file"], f.get("line") or 0)

    defects = sorted([f for f in live if f["final_category"] in DEFECT_CATS], key=sort_key)
    quality = sorted([f for f in live if f["final_category"] not in DEFECT_CATS], key=sort_key)

    n = 0
    for f in defects + quality:
        n += 1
        f["rid"] = f"RA-{n:03d}"

    out = []
    out.append(open(cfg["header"]).read().rstrip() + "\n")

    def stats_table(items):
        rows = Counter((f["area"], f["final_severity"]) for f in items)
        areas = sorted({f["area"] for f in items})
        t = ["| Area | Critical | High | Medium | Low | Total |", "| --- | ---: | ---: | ---: | ---: | ---: |"]
        for a in areas:
            c = [rows[(a, s)] for s in ("critical", "high", "medium", "low")]
            t.append(f"| {a} | " + " | ".join(str(x) for x in c) + f" | {sum(c)} |")
        tot = [sum(rows[(a, s)] for a in areas) for s in ("critical", "high", "medium", "low")]
        t.append("| **All** | " + " | ".join(f"**{x}**" for x in tot) + f" | **{sum(tot)}** |")
        return "\n".join(t)

    out.append("## Findings at a glance\n")
    out.append("### Defects and security\n")
    out.append(stats_table(defects) + "\n")
    out.append("### Code quality and design\n")
    out.append(stats_table(quality) + "\n")
    cats = Counter(f["final_category"] for f in live)
    out.append("By category: " + ", ".join(f"{CAT_LABEL[c]} {k}" for c, k in cats.most_common()) + ".\n")

    def index_table(items):
        t = ["| ID | Severity | Category | Location | Title |", "| --- | --- | --- | --- | --- |"]
        for f in items:
            loc = f"{f['file']}:{f.get('line')}"
            t.append(f"| {f['rid']} | {f['final_severity']} | {CAT_LABEL[f['final_category']]} | `{loc}` | {md_escape_title(f['title']).replace('|', '/')} |")
        return "\n".join(t)

    top = [f for f in defects + quality if f["final_severity"] in ("critical", "high")]
    out.append("## Critical and high findings index\n")
    out.append(index_table(top) + "\n")

    for part_title, items in (("Part A: Defects and security", defects), ("Part B: Code quality and design", quality)):
        out.append(f"## {part_title}\n")
        for sev in ("critical", "high", "medium", "low"):
            group = [f for f in items if f["final_severity"] == sev]
            if not group:
                continue
            out.append(f"### {sev.capitalize()} ({len(group)})\n")
            for f in group:
                out.append(render_finding(f, f["rid"]))

    for i, f in enumerate(sorted(disputed, key=lambda f: (f["file"], f.get("line") or 0)), start=1):
        f["rid"] = f"RD-{i:02d}"
    out.append("## Appendix A: Disputed high-severity claims\n")
    if disputed:
        out.append("Earlier verifiers confirmed each of these and the blind round-2 re-check refuted it. They are listed with both arguments so nothing is silently dropped; read them as leads to settle, not as findings.\n")
    else:
        out.append("None. The blind round-2 re-check refuted no finding that earlier verifiers had confirmed.\n")
    for f in sorted(disputed, key=lambda f: f["rid"]):
        out.append(render_finding(f, f["rid"]))
    if fixed:
        out.append("## Appendix B: Fixed since the audited commit\n")
        out.append("PR #30 and PR #31 changed these lines after commit 361c6f9; a recheck found the problem gone.\n")
        for f in sorted(fixed, key=lambda f: (f["file"], f.get("line") or 0)):
            out.append(f"- {md_escape_title(f['title'])} (`{f['file']}:{f.get('line')}`, was {f['final_severity']}): {(f.get('recheck_reason') or '').strip()}\n")
    letter = "C"
    if unverified:
        out.append("## Appendix C: Unverified findings\n")
        letter = "D"
        out.append("Their verification agents failed; they have not been checked.\n")
        for f in unverified:
            out.append(f"- {md_escape_title(f['title'])} (`{f['file']}:{f.get('line')}`, {f['severity']}, {f['category']})\n")
    out.append(f"## Appendix {letter}: Coverage notes from the auditors\n")
    for unit, note in coverage_notes(cfg["rounds"]):
        note = " ".join(note.split())
        out.append(f"- **{unit}**: {note}\n")

    open(cfg["out_md"], "w").write("\n".join(out))

    data = {
        "findings": [
            {k: f.get(k) for k in ("rid", "title", "file", "line", "symbol", "area", "final_severity", "final_category",
                                    "status", "confidence", "effort", "evidence", "impact", "fix", "related",
                                    "merged_from", "unit", "round", "recheck", "dispute")}
            | {"notes": corrections(f)}
            for f in defects + quality + sorted(disputed, key=lambda f: f["rid"])
        ],
        "disputed": [
            {"title": f["title"], "file": f["file"], "line": f.get("line"),
             "claimed": (f.get("v1") or {}).get("severity"),
             "for": (f.get("v1") or {}).get("reason"), "against": (f.get("v2") or {}).get("reason")}
            for f in disputed
        ],
        "counts": {"live": len(live), "defects": len(defects), "quality": len(quality), "disputed": len(disputed),
                   "refuted": refuted, "unverified": len(unverified), "fixed": len(fixed), "candidates": len(allf), "merged": len(dropped)},
    }
    json.dump(data, open(cfg["out_json"], "w"), indent=1)
    print(json.dumps(data["counts"]))


if __name__ == "__main__":
    main()
