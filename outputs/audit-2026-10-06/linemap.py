"""Map 361c6f9 line numbers to d04b526 for findings in changed files.

Usage: python3 -I linemap.py <repo> <changed_files.txt> <findings.json> <out.json>
findings.json: [{id, file, line}]. Output: {id: {new_line, touched}} where touched means a hunk
edits or deletes lines within 12 lines of the finding's line.
"""
import json
import re
import subprocess
import sys

repo, changed, fpath, out = sys.argv[1:5]
changed = set(open(changed).read().split())
fs = json.load(open(fpath))
hunks = {}
for path in changed:
    diff = subprocess.run(["git", "-C", repo, "diff", "-U0", "361c6f9", "d04b526", "--", path],
                          capture_output=True, text=True, timeout=60).stdout
    hs = []
    for m in re.finditer(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", diff, re.M):
        os_, oc = int(m.group(1)), int(m.group(2) or 1)
        ns, nc = int(m.group(3)), int(m.group(4) or 1)
        hs.append((os_, oc, ns, nc))
    hunks[path] = hs

res = {}
for f in fs:
    if f["file"] not in changed:
        continue
    line = f.get("line") or 0
    shift, touched = 0, False
    for os_, oc, ns, nc in hunks[f["file"]]:
        lo, hi = (os_, os_ + oc - 1) if oc else (os_, os_)
        if lo - 12 <= line <= hi + 12:
            touched = True
        if (oc and os_ + oc - 1 < line) or (not oc and os_ < line):
            shift += nc - oc
    res[f["id"]] = {"file": f["file"], "old_line": line, "new_line": line + shift, "touched": touched}
json.dump(res, open(out, "w"), indent=1)
print(sum(1 for r in res.values() if r["touched"]), "touched of", len(res))
