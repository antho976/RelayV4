"""Fold round 2 into the round-1 data, producing round2-merged.json for build_report.py.

Usage: python3 -I merge_r2.py round1.json round2-dedup.json round2-completeness.json \
           round2-blind.json round2-linemap.json round2-merged.json

- round2-blind.json: {blind:[{id, verdict, severity, category, reason, correction, line}],
                      recheck:[{id, state, line, reason, correction}]}
  A blind verdict replaces the earlier one as final. A blind refutation of a finding that earlier
  verifiers confirmed makes it "disputed", with every verifier's argument kept.
  A recheck (findings near code PR #30/#31 changed) marks it "fixed" or updates its reading.
- round2-linemap.json: {id: {new_line}} for findings in files changed after 361c6f9; lines in the
  output refer to the current tree (d04b526), which equals 361c6f9 everywhere else.
- round2-completeness.json: the completeness pass ({mined:[{findings:[... with v1]}]}).
"""
import json
import sys

r1, dedup_p, comp_p, blind_p, linemap_p, out = sys.argv[1:7]
data = json.load(open(r1))
byid = {f["id"]: f for u in data["results"] for f in u["findings"]}
blind = json.load(open(blind_p))
linemap = json.load(open(linemap_p))

for fid, m in linemap.items():
    if fid in byid:
        byid[fid]["line"] = m["new_line"]

for v in blind.get("recheck", []):
    f = byid[v["id"]]
    f["v_recheck"] = v
    if v["state"] == "fixed":
        f["status"] = "fixed"
        f["recheck_reason"] = v["reason"]
    else:
        f["line"] = v["line"] or f["line"]
        f["recheck"] = "rechecked after PR #30/#31: " + ("changed" if v["state"] == "changed" else "still present")


def reason(v):
    return ((v or {}).get("reason") or "").strip()


for v in blind["blind"]:
    f = byid[v["id"]]
    f["v3"] = v
    if v["verdict"] == "refuted":
        args = [f"Round-1 verifier ({(f.get('v1') or {}).get('verdict')}, {(f.get('v1') or {}).get('severity')}): {reason(f.get('v1'))}"]
        if f.get("v2"):
            args.append(f"Round-1 skeptic ({f['v2'].get('verdict')}, {f['v2'].get('severity')}): {reason(f['v2'])}")
        args.append(f"Round-2 blind re-check (refuted): {reason(v)}")
        f["dispute"] = "\n\n".join(args)
        f["status"] = "disputed"
    else:
        f["status"] = v["verdict"]
        f["final_severity"] = v["severity"]
        f["final_category"] = v["category"]
        if v.get("line"):
            f["line"] = v["line"]
        f["blind"] = True

comp = json.load(open(comp_p))
new = []
for m in comp["mined"]:
    for f in m["findings"]:
        v1 = f.get("v1")
        f = dict(f)
        if not v1:
            f["status"], f["final_severity"], f["final_category"] = "unverified", f["severity"], f["category"]
        else:
            f["status"] = v1["verdict"]
            f["final_severity"], f["final_category"] = v1["severity"], v1["category"]
        new.append(f)
data["results"].append({"unit": "r2-completeness", "kind": "completeness",
                        "coverage": "Round-2 completeness pass: issues round-1 auditors noted outside their chunk and did not report, written up and verified.",
                        "findings": new})

dedup = json.load(open(dedup_p))
dropped = {d for g in dedup["groups"] for d in g["dupes"]}
live = [f for u in data["results"] for f in u["findings"] if f["id"] not in dropped]
count = lambda s: sum(1 for f in live if f["status"] == s)
data["summary"] = {
    "round1": data.get("summary"),
    "round2": {
        "merged_duplicates": len(dropped), "groups": len(dedup["groups"]),
        "blind_checked": len(blind["blind"]),
        "blind_verdicts": {k: sum(1 for v in blind["blind"] if v["verdict"] == k) for k in ("confirmed", "plausible", "refuted")},
        "rechecked": len(blind.get("recheck", [])), "fixed_since": count("fixed"),
        "completeness_new": len(new), "completeness_kept": sum(1 for f in new if f["status"] in ("confirmed", "plausible")),
        "unique_live": count("confirmed") + count("plausible"), "disputed": count("disputed"),
    },
}
json.dump(data, open(out, "w"), indent=1)
print(json.dumps(data["summary"]["round2"]))
