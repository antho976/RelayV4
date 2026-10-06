"""Render the report data (build_report.py's out_json) as one self-contained, filterable HTML page.

Usage: python3 -I build_html.py <report-data.json> <out.html> [title]
"""
import html
import json
import sys

data = json.load(open(sys.argv[1]))
out = sys.argv[2]
title = sys.argv[3] if len(sys.argv) > 3 else "RelayV4 code audit"

data.setdefault("subtitle", f"{data['counts']['live']} unique findings from the first full audit of RelayV4 "
                            "(audited at 361c6f9, lines at d04b526). High findings shown by default; "
                            "use the chips to add medium and low.")
payload = json.dumps(data, separators=(",", ":")).replace("</", "<\\/")

PAGE = r"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>__TITLE__</title>
<style>
:root {
  --bg: #fbfaf8; --panel: #ffffff; --ink: #1d1c1a; --muted: #6b6760; --line: #e4e0d8;
  --accent: #3b5bdb; --chip: #f1eee8;
  --crit: #b42318; --high: #c4320a; --med: #b54708; --low: #4b5563;
  --crit-bg: #fee4e2; --high-bg: #ffead5; --med-bg: #fef0c7; --low-bg: #eef0f3;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: #161514; --panel: #1f1e1c; --ink: #ece9e3; --muted: #a39e94; --line: #34312d;
    --accent: #8ea2ff; --chip: #2a2825;
    --crit: #ff8a80; --high: #ffab70; --med: #f5c26b; --low: #b8bfca;
    --crit-bg: #4a1d1a; --high-bg: #4a2a14; --med-bg: #3f3214; --low-bg: #2a2d33;
  }
}
:root[data-theme="dark"] {
  --bg: #161514; --panel: #1f1e1c; --ink: #ece9e3; --muted: #a39e94; --line: #34312d;
  --accent: #8ea2ff; --chip: #2a2825;
  --crit: #ff8a80; --high: #ffab70; --med: #f5c26b; --low: #b8bfca;
  --crit-bg: #4a1d1a; --high-bg: #4a2a14; --med-bg: #3f3214; --low-bg: #2a2d33;
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--ink); font: 15px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }
header { padding: 24px 16px 8px; max-width: 1100px; margin: 0 auto; }
h1 { font-size: 22px; margin: 0 0 4px; }
.sub { color: var(--muted); margin: 0; }
.controls { position: sticky; top: 0; z-index: 2; background: var(--bg); border-bottom: 1px solid var(--line); }
.controls .inner { max-width: 1100px; margin: 0 auto; padding: 10px 16px; display: flex; flex-wrap: wrap; gap: 8px; align-items: center; }
input[type=search], select { font: inherit; color: var(--ink); background: var(--panel); border: 1px solid var(--line); border-radius: 6px; padding: 6px 8px; }
input[type=search] { flex: 1 1 220px; min-width: 0; }
select { flex: 0 1 auto; max-width: 100%; }
.count { color: var(--muted); font-size: 13px; margin-left: auto; }
main { max-width: 1100px; margin: 0 auto; padding: 12px 16px 64px; }
.sevbar { display: flex; flex-wrap: wrap; gap: 6px; }
.sevbtn { font: inherit; font-size: 13px; border: 1px solid var(--line); background: var(--panel); color: var(--ink); border-radius: 999px; padding: 3px 10px; cursor: pointer; }
.sevbtn[aria-pressed=true] { border-color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); }
details.f { background: var(--panel); border: 1px solid var(--line); border-radius: 8px; margin: 8px 0; }
details.f > summary { list-style: none; cursor: pointer; padding: 10px 12px; display: grid; grid-template-columns: auto auto 1fr; gap: 4px 10px; align-items: baseline; }
details.f > summary::-webkit-details-marker { display: none; }
.rid { font: 12px ui-monospace, SFMono-Regular, Menlo, monospace; color: var(--muted); }
.sev { font-size: 12px; font-weight: 600; padding: 1px 8px; border-radius: 999px; text-transform: uppercase; letter-spacing: .02em; }
.sev.critical { color: var(--crit); background: var(--crit-bg); }
.sev.high { color: var(--high); background: var(--high-bg); }
.sev.medium { color: var(--med); background: var(--med-bg); }
.sev.low { color: var(--low); background: var(--low-bg); }
.t { font-weight: 550; overflow-wrap: anywhere; }
.meta { grid-column: 1 / -1; color: var(--muted); font-size: 13px; overflow-wrap: anywhere; }
.meta code { font-size: 12px; }
.body { padding: 0 12px 12px; border-top: 1px solid var(--line); }
.body h4 { font-size: 13px; text-transform: uppercase; letter-spacing: .04em; color: var(--muted); margin: 12px 0 2px; }
.body p { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; }
code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; background: var(--chip); padding: 0 4px; border-radius: 4px; }
.tag { display: inline-block; font-size: 12px; background: var(--chip); border-radius: 4px; padding: 0 6px; margin-right: 4px; }
.empty { color: var(--muted); padding: 32px 0; text-align: center; }
@media (max-width: 600px) { details.f > summary { grid-template-columns: auto 1fr; } .rid { grid-column: 1 / -1; } }
</style>
</head>
<body>
<header>
  <h1>__TITLE__</h1>
  <p class="sub" id="sub"></p>
</header>
<div class="controls"><div class="inner">
  <div class="sevbar" id="sevbar"></div>
  <input type="search" id="q" placeholder="Search title, file, text…" aria-label="Search">
  <select id="area" aria-label="Area"></select>
  <select id="cat" aria-label="Category"></select>
  <select id="part" aria-label="Part">
    <option value="">Both parts</option><option value="A">A: defects and security</option><option value="B">B: quality and design</option>
  </select>
  <select id="status" aria-label="Status">
    <option value="">Any status</option><option value="confirmed">Confirmed</option><option value="plausible">Plausible</option><option value="disputed">Disputed</option>
  </select>
  <span class="count" id="count"></span>
</div></div>
<main id="list"></main>
<script id="data" type="application/json">__DATA__</script>
<script>
const D = JSON.parse(document.getElementById('data').textContent);
const DEFECT = new Set(['bug','security','concurrency','invariant','performance','resource-leak','error-handling','data-loss']);
const SEVS = ['critical','high','medium','low'];
const items = D.findings.map(f => ({...f, part: DEFECT.has(f.final_category) ? 'A' : 'B',
  hay: [f.rid, f.title, f.file, f.symbol, f.evidence, f.impact, f.fix, (f.related||[]).join(' '), (f.merged_from||[]).join(' ')].join(' ').toLowerCase()}));
const sevOn = new Set(['critical','high']);
let state = {q:'', area:'', cat:'', part:'', status:''};
try { const s = JSON.parse(localStorage.getItem('audit-filters')||'null'); if (s) { state = {...state, ...s.state}; sevOn.clear(); s.sev.forEach(x => sevOn.add(x)); } } catch (e) {}
function save() { try { localStorage.setItem('audit-filters', JSON.stringify({state, sev:[...sevOn]})); } catch (e) {} }
const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
const md = s => esc(s).replace(/`([^`]+)`/g, '<code>$1</code>');
function opts(el, label, values) {
  el.innerHTML = `<option value="">${label}</option>` + values.map(v => `<option>${esc(v)}</option>`).join('');
}
const counts = {}; items.forEach(f => counts[f.final_severity] = (counts[f.final_severity]||0)+1);
document.getElementById('sub').textContent = D.subtitle || `${items.length} findings`;
const bar = document.getElementById('sevbar');
SEVS.forEach(s => { const b = document.createElement('button'); b.className = 'sevbtn'; b.dataset.s = s;
  b.textContent = `${s[0].toUpperCase()+s.slice(1)} ${counts[s]||0}`; b.setAttribute('aria-pressed', sevOn.has(s));
  b.onclick = () => { sevOn.has(s) ? sevOn.delete(s) : sevOn.add(s); b.setAttribute('aria-pressed', sevOn.has(s)); render(); }; bar.appendChild(b); });
opts(document.getElementById('area'), 'All areas', [...new Set(items.map(f => f.area))].sort());
opts(document.getElementById('cat'), 'All categories', [...new Set(items.map(f => f.final_category))].sort());
for (const k of ['area','cat','part','status']) { const el = document.getElementById(k); el.value = state[k]; el.onchange = () => { state[k] = el.value; render(); }; }
const q = document.getElementById('q'); q.value = state.q; q.oninput = () => { state.q = q.value; render(); };
function card(f) {
  const loc = f.line ? `${f.file}:${f.line}` : f.file;
  const tags = [f.final_category, f.status, 'effort ' + (f.effort||'?')];
  if (f.recheck) tags.push(f.recheck);
  let b = `<h4>Problem</h4><p>${md(f.evidence)}</p><h4>Impact</h4><p>${md(f.impact)}</p><h4>Fix</h4><p>${md(f.fix)}</p>`;
  (f.notes||[]).forEach(n => b += `<h4>Verifier note</h4><p>${md(n)}</p>`);
  if (f.dispute) b += `<h4>Disputed</h4><p>${md(f.dispute)}</p>`;
  if ((f.related||[]).length) b += `<h4>Also at</h4><p>${f.related.map(r => `<code>${esc(r)}</code>`).join(', ')}</p>`;
  if ((f.merged_from||[]).length) b += `<h4>Merged duplicates</h4><p>${esc(f.merged_from.join(', '))}</p>`;
  return `<details class="f"><summary><span class="rid">${esc(f.rid)}</span><span class="sev ${f.final_severity}">${f.final_severity}</span><span class="t">${md(f.title)}</span>
    <span class="meta"><code>${esc(loc)}</code> · ${esc(f.area)} · ${tags.map(t => `<span class="tag">${esc(t)}</span>`).join('')}</span></summary><div class="body">${b}</div></details>`;
}
function render() {
  save();
  const needle = state.q.trim().toLowerCase();
  const shown = items.filter(f => sevOn.has(f.final_severity) && (!state.area || f.area === state.area) &&
    (!state.cat || f.final_category === state.cat) && (!state.part || f.part === state.part) &&
    (!state.status || f.status === state.status) && (!needle || f.hay.includes(needle)));
  document.getElementById('count').textContent = `${shown.length} of ${items.length}`;
  const list = document.getElementById('list');
  list.innerHTML = shown.length ? shown.slice(0, 400).map(card).join('') + (shown.length > 400 ? `<p class="empty">Showing the first 400; narrow the filters to see the rest.</p>` : '')
    : '<p class="empty">No findings match these filters.</p>';
}
render();
</script>
</body>
</html>
"""

page = PAGE.replace("__TITLE__", html.escape(title)).replace("__DATA__", payload)
open(out, "w").write(page)
print(out, len(page))
