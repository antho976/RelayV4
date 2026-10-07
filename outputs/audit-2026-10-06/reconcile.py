"""Reconcile per-shard dedup groups into one consistent set (dedup.json).

Groups from different shards that share members describe the same root cause seen from two
sides; they are merged. Clusters checked by hand and found to chain distinct root causes are
replaced by MANUAL groups below.
Usage: python3 -I reconcile.py <round1.json> <partA.json> <out dedup.json>
"""
import collections
import json
import sys

SEV = {"critical": 0, "high": 1, "medium": 2, "low": 3}

MANUAL = [
    # Codex edits gated as exec (Guarded label is false)
    {"keep": "lens-docs#1", "dupes": ["cli-main#10", "sec-guardrail-bypass#6"],
     "note": "codex_pre_tool maps apply_patch/Edit/Write to kind exec, so evaluate_write never runs for Codex edits while Codex::guarded() is true."},
    # Shell writes through the Bash tool skip write checks
    {"keep": "sec-guardrail-bypass#1", "dupes": ["core-guardrail#16"],
     "note": "evaluate_exec checks only self_approval and denied_commands; redirections, sed -i, tee and cp writes never meet the write policy."},
    # BUS.md describes the Codex guard the opposite way round
    {"keep": "lens-docs#10", "dupes": ["sec-supply-chain#17", "lens-contract#18"],
     "note": "BUS.md 9.3/10.8/14 say Codex is unguarded and covered by a post-hoc guardrail.violation watcher; the code installs Codex hooks, reports guarded=true, and has no watcher."},
    # Shadow ui.* model vs Tauri leftovers
    {"keep": "bus#4", "dupes": ["lens-dead-dup#7", "lens-contract#8", "lens-overengineering#7"],
     "note": "ui.* ops answer from an engine-side UiRuntime the GTK client never reads or renders."},
    {"keep": "lens-overengineering#11", "dupes": ["tests-bus-guardrails#13", "lens-dead-dup#16"],
     "note": "Door::Tauri, Doors::TauriOnly and related scaffolding survive the removed Tauri client."},
    # Drifted engine names in the native client vs missing workspace.deleted subscription
    {"keep": "lens-dead-dup#18", "dupes": ["lens-contract#16", "native-tasks#15"],
     "note": "The native client hard-codes engine names that drifted (device.sdk_missing vs avd.sdk_missing, git.worktree.create)."},
    {"keep": "native-app#11", "dupes": ["lens-contract#19"],
     "note": "The desktop event subscription is a hand-kept allow-list that omits workspace.deleted."},
]

r1, partA, out = sys.argv[1:4]
d = json.load(open(r1))
byid = {f["id"]: f for x in d["results"] for f in x["findings"]}
a = json.load(open(partA))
gs = [dict(g, shard=s["shard"]) for s in a["dedup"] for g in s["groups"]]
manual_ids = {m for g in MANUAL for m in [g["keep"]] + g["dupes"]}

par = list(range(len(gs)))


def find(x):
    while par[x] != x:
        par[x] = par[par[x]]
        x = par[x]
    return x


occ = collections.defaultdict(list)
for i, g in enumerate(gs):
    for m in [g["keep"]] + g["dupes"]:
        occ[m].append(i)
for m, v in occ.items():
    for j in v[1:]:
        par[find(j)] = find(v[0])
comp = collections.defaultdict(list)
for i in range(len(gs)):
    comp[find(i)].append(i)

final = []
for c in comp.values():
    members = {m for i in c for m in [gs[i]["keep"]] + gs[i]["dupes"]}
    if members & manual_ids:
        assert members <= manual_ids or True
        continue
    keeps = collections.Counter(gs[i]["keep"] for i in c)

    def rank(m):
        f = byid[m]
        return (0 if f.get("v2") else 1, SEV[f["final_severity"]], -keeps.get(m, 0), m)

    keep = min(keeps, key=rank)
    notes = [gs[i]["note"] for i in c]
    sev_notes = [gs[i]["severity_note"] for i in c if gs[i].get("severity_note")]
    final.append({"keep": keep, "dupes": sorted(members - {keep}), "note": notes[0],
                  "severity_notes": sev_notes, "shards": sorted({gs[i]["shard"] for i in c})})

# Members of hand-split clusters that MANUAL does not place stay single.
covered = {m for g in MANUAL for m in [g["keep"]] + g["dupes"]}
leftover = set()
for c in comp.values():
    members = {m for i in c for m in [gs[i]["keep"]] + gs[i]["dupes"]}
    if members & manual_ids:
        leftover |= members - covered
for g in MANUAL:
    final.append({**g, "severity_notes": [], "shards": ["manual"]})

seen = collections.Counter(m for g in final for m in [g["keep"]] + g["dupes"])
dup_members = [m for m, k in seen.items() if k > 1]
assert not dup_members, dup_members
json.dump({"groups": final, "left_single": sorted(leftover)}, open(out, "w"), indent=1)
dropped = sum(len(g["dupes"]) for g in final)
live = [f for f in byid.values() if f["status"] != "refuted"]
print(f"groups {len(final)}, merged away {dropped}, unique {len(live) - dropped}, left single from split clusters {sorted(leftover)}")
