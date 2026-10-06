export const meta = {
  name: 'audit-r2-blind-recheck',
  description: 'Audit round 2, part 2: one blind verifier per unique high/critical finding (63), and a recheck of findings near code PR #30/#31 changed',
  phases: [
    { title: 'Blind', detail: 'fresh verifier per high/critical finding, no earlier verdicts' },
    { title: 'Recheck', detail: 'findings near PR #30/#31 edits' },
  ],
}

const ROOT = '/home/anthony/dev3/RelayV4/.relay/worktrees/quick-moose'
const SCRATCH = '/tmp/claude-1000/-home-anthony-dev3-RelayV4--relay-worktrees-quick-moose/fffe0615-966d-4b84-9eae-53aa00f95693/scratchpad'
const INPUT = SCRATCH + '/r2/partB_args.json'
const CATS = ['bug', 'security', 'concurrency', 'invariant', 'performance', 'resource-leak', 'error-handling', 'data-loss', 'dead-code', 'duplication', 'overengineering', 'code-quality', 'test-quality', 'docs-drift']
const SEV = ['critical', 'high', 'medium', 'low']

const BLIND = {
  type: 'object',
  properties: {
    id: { type: 'string' },
    verdict: { type: 'string', enum: ['confirmed', 'plausible', 'refuted'] },
    severity: { type: 'string', enum: SEV },
    category: { type: 'string', enum: CATS },
    reason: { type: 'string', description: 'your answers to (a)-(e): what the code does, who triggers it and how often, what happens, why this severity, whether the fix is right; for refuted, exactly what the finding got wrong' },
    correction: { type: 'string', description: 'corrections to lines, mechanism, impact or fix; empty string if none' },
    line: { type: 'integer', description: 'the 1-indexed line of the decisive code in the current tree' },
  },
  required: ['id', 'verdict', 'severity', 'category', 'reason', 'correction', 'line'],
}
const RECHECK = {
  type: 'object',
  properties: {
    results: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          state: { type: 'string', enum: ['still-present', 'changed', 'fixed'] },
          line: { type: 'integer', description: 'current 1-indexed line (0 if fixed)' },
          reason: { type: 'string', description: 'what the diff did to the problem' },
          correction: { type: 'string', description: 'for changed: how the evidence, impact or fix now reads; empty otherwise' },
        },
        required: ['id', 'state', 'line', 'reason', 'correction'],
      },
    },
  },
  required: ['results'],
}

const BASE = `Context: RelayV4 is a Rust workspace: a local engine (crates/relay-core: one SQLite store behind one mutex, a typed command bus over a Unix socket, PTY-backed AI-agent sessions in git worktrees, guardrails that gate what agents may run/write, Android device and emulator tools, scrcpy mirroring), the bus types (crates/relay-bus), the CLI and MCP servers (crates/relay-cli, including Unreal/Blender tooling that runs embedded Python), the phone door (crates/relay-remote: LAN listener, rendezvous relay, tunnel, pairing), and a GTK4 desktop client (apps/relay-native). The phone app (apps/relay-mobile) is out of scope.
Repository (your cwd): ${ROOT}. Architecture references: docs/ARCHITECTURE.md, docs/engine/BUS.md, docs/engine/SPEC.md, docs/engine/DECISIONS.md (code comments cite these as §n and Dnnn). CLAUDE.md lists invariants that are easy to break.

HARD RULES
- Strictly read-only in the repository. Do not modify, create, delete, stage or commit anything. Do not run cargo, rustc, npm or the app. Do not call any mcp__relay__* tool: they act on the user's live Relay engine.
- Use Read, Grep, Glob and read-only shell (rg, git log/show/blame/diff, jq, python3 -I). Throwaway files go under ${SCRATCH}/exp/ only.

SEVERITY (real-world impact: a single-user desktop app with a local engine, many concurrent agent sessions, and an optional phone door reachable over LAN or a rendezvous server)
- critical: data loss/corruption; a security hole reachable by another local user, a network peer, or an agent escaping its guardrails; an engine crash/hang that takes down every session; a frequent hard failure of a core flow.
- high: likely user-visible wrong behavior or failure in a normal flow; a freeze of the bus or the UI; unbounded growth in normal use; a safety check that can be silently bypassed.
- medium: a real bug in a less common path or edge case; a design/maintainability problem with a concrete ongoing cost (e.g. duplicated logic that has already diverged).
- low: a minor bug in a rare edge case; a clear simplification or cleanup with modest payoff.
CATEGORIES: bug, security, concurrency, invariant (breaks a CLAUDE.md rule: slow work in a handler registered with Engine::register while holding the store mutex, a subprocess not run through proc::output_with_timeout, session.input touching SQLite, migration rules), performance, resource-leak, error-handling, data-loss, dead-code, duplication, overengineering, code-quality, test-quality, docs-drift.`

function blindPrompt(id) {
  return `${BASE}

YOUR ROLE: independent verifier of one claim from a code audit. The claim is the object whose "id" is "${id}" in the "blind" array of ${INPUT}; read it with: jq '.blind[] | select(.id=="${id}")' ${INPUT}
It gives the auditor's title, location, evidence, impact and fix, plus "same_root_cause_reports": other auditors' reports that a reviewer judged to be the same root cause (their titles show other angles and call sites of the problem).
You are deliberately NOT shown anyone else's verdict or severity rating, and you must not look for them: do not open anything under outputs/ (no round1.json, index.tsv, report or coverage files) and do not search the repository for this id.

High-severity claims carry an audit report, and a wrong one destroys its credibility. Default to refuting unless you establish the claim yourself from the code. Open the cited code and read enough surrounding code, callers and docs to judge independently; do not trust the claim's quotes or line numbers. Answer for yourself:
(a) Does the code do what is claimed?
(b) Is the trigger reachable in real use of this app, by whom, and how often? Look for upstream guards, validation, transactions, locks, retries and documented decisions that defuse it.
(c) What exactly happens to the user or their data when it fires?
(d) What severity does the rubric give, rated fresh from (b) and (c)? Consider the worst real angle among the same-root-cause reports, not just the main one.
(e) Is the proposed fix correct and complete, or would it break something?
Verdict: "confirmed" (you established it yourself, possibly with corrections), "plausible" (probably real, but you could not fully establish the trigger or impact), or "refuted" (wrong, unreachable, already handled, or intended and harmless). Between plausible and refuted, choose refuted.
Put the answers to (a)-(e) in "reason" and any corrections in "correction". Return your verdict for id ${id} via StructuredOutput.`
}

function recheckPrompt(ids) {
  return `${BASE}

YOUR ROLE: recheck audit findings after a merge. The findings were verified at commit 361c6f9. Since then PR #30 and PR #31 changed the code near them (the current tree is commit d04b526 plus audit docs). Each finding is an object in the "recheck" array of ${INPUT}; read yours with: jq '.recheck[] | select(.id as $i | ${JSON.stringify(ids)} | index($i))' ${INPUT}
Ids: ${ids.join(', ')}. "old_line" is at 361c6f9; "new_line" is a mechanical estimate for the current tree.
For each finding, read \`git diff 361c6f9 d04b526 -- <file>\` and the current code, and decide:
- "still-present": the problem exists unchanged (give its current line);
- "changed": the problem still exists but the mechanism, impact or right fix changed (give the current line and the corrected reading in "correction");
- "fixed": the change removed the problem (line 0; say how in "reason").
Judge only whether the change affected the finding; do not re-litigate a finding the change did not touch (mark it still-present). Return via StructuredOutput.`
}

const blindIds = args.blind
const recheckGroups = args.recheck

const blindP = parallel(blindIds.map(id => async () => {
  for (let attempt = 0; attempt < 2; attempt++) {
    const v = await agent(blindPrompt(id), { label: `blind:${id}${attempt ? 'r' : ''}`, phase: 'Blind', schema: BLIND, effort: 'high' })
    if (v) return { ...v, id }
  }
  return null
}))
const recheckP = parallel(recheckGroups.map((ids, i) => async () => {
  const r = await agent(recheckPrompt(ids), { label: `recheck:${i + 1}`, phase: 'Recheck', schema: RECHECK, effort: 'high' })
  return r ? r.results.filter(x => ids.includes(x.id)) : null
}))
const [blind, recheck] = await Promise.all([blindP, recheckP])
const missing = blindIds.filter((id, i) => !blind[i])
if (missing.length) log(`blind verdict missing for ${missing.join(', ')}`)
const counts = {}
blind.filter(Boolean).forEach(v => { const k = `${v.verdict}/${v.severity}`; counts[k] = (counts[k] || 0) + 1 })
log(`blind: ${JSON.stringify(counts)}`)
return { blind: blind.filter(Boolean), missing, recheck: recheck.filter(Boolean).flat() }
