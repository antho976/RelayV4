"""Fill report-page.html with the report data (build_report.py's out_json).

Usage: python3 -I build_html.py <report-data.json> <out.html> [--standalone]

Without --standalone the output is the page body as claude.ai artifacts expect it (the host adds
the doctype, head and body). With it, the page is wrapped so it opens directly in a browser.
"""
import json
import os
import sys

src, out = sys.argv[1], sys.argv[2]
standalone = "--standalone" in sys.argv[3:]
here = os.path.dirname(os.path.abspath(__file__))
data = json.load(open(src))

KEEP = ("rid", "title", "file", "line", "symbol", "area", "final_severity", "final_category", "status",
        "effort", "evidence", "impact", "fix", "related", "merged_from", "notes", "recheck")
slim = {"findings": [{k: f[k] for k in KEEP if f.get(k)} for f in data["findings"]], "counts": data["counts"]}
payload = json.dumps(slim, separators=(",", ":")).replace("</", "<\\/").replace("<!--", "<\\!--")

page = open(os.path.join(here, "report-page.html")).read().replace("__DATA__", payload)
if standalone:
    page = ('<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n'
            '<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">\n'
            '</head>\n<body>\n' + page + '\n</body>\n</html>\n')
open(out, "w").write(page)
print(out, len(page))
