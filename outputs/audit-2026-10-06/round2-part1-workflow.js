export const meta = {
  name: 'audit-r2-dedup-completeness',
  description: 'Audit round 2, part 1: semantic dedup of 1,103 round-1 findings in 11 shards, and a completeness pass over coverage.md with write-up and verification of uncovered issues',
  phases: [
    { title: 'Dedup', detail: 'one agent per shard of round-1 findings' },
    { title: 'Mine', detail: 'two agents read coverage.md for unreported issues' },
    { title: 'Write-up', detail: 'turn uncovered issues into findings' },
    { title: 'Verify', detail: 'adversarial verifier per batch of new findings' },
  ],
}

const ROOT = '/home/anthony/dev3/RelayV4/.relay/worktrees/quick-moose'
const AUD = ROOT + '/outputs/audit-2026-10-06'
const SCRATCH = '/tmp/claude-1000/-home-anthony-dev3-RelayV4--relay-worktrees-quick-moose/fffe0615-966d-4b84-9eae-53aa00f95693/scratchpad'
const IN = SCRATCH + '/r2/in'
const SHARDS = ['bus-cli-remote-misc-1', 'bus-cli-remote-misc-2', 'engine-core-1', 'engine-core-2', 'handlers-core', 'handlers-other-1', 'handlers-other-2', 'native-main', 'native-other-1', 'native-other-2', 'tests-perf']
const CATS = ['bug', 'security', 'concurrency', 'invariant', 'performance', 'resource-leak', 'error-handling', 'data-loss', 'dead-code', 'duplication', 'overengineering', 'code-quality', 'test-quality', 'docs-drift']
const SEV = ['critical', 'high', 'medium', 'low']

const FINDING = {
  type: 'object',
  properties: {
    title: { type: 'string', description: 'One specific line: what is wrong and where' },
    category: { type: 'string', enum: CATS },
    severity: { type: 'string', enum: SEV },
    confidence: { type: 'string', enum: ['high', 'medium', 'low'] },
    file: { type: 'string', description: 'repo-relative path' },
    line: { type: 'integer', description: '1-indexed line of the decisive code' },
    symbol: { type: 'string', description: 'function / type / selector involved' },
    related: { type: 'array', items: { type: 'string' }, description: 'other file:line locations (callers, repeats, duplicates)' },
    evidence: { type: 'string', description: 'what the code does, quoting the decisive lines' },
    impact: { type: 'string', description: 'concrete trigger -> consequence (defects) or concrete ongoing cost (quality)' },
    fix: { type: 'string', description: 'specific recommended change; for overengineering, the simpler design and what it removes' },
    effort: { type: 'string', enum: ['S', 'M', 'L'] },
  },
  required: ['title', 'category', 'severity', 'confidence', 'file', 'line', 'evidence', 'impact', 'fix', 'effort'],
}
const WRITEUP = {
  type: 'object',
  properties: {
    findings: { type: 'array', items: { type: 'object', properties: { cand: { type: 'string', description: 'candidate key this finding came from' }, finding: FINDING }, required: ['cand', 'finding'] } },
    dropped: { type: 'array', items: { type: 'object', properties: { cand: { type: 'string' }, why: { type: 'string' } }, required: ['cand', 'why'] } },
  },
  required: ['findings', 'dropped'],
}
const VERDICT = {
  type: 'object',
  properties: {
    id: { type: 'string' },
    verdict: { type: 'string', enum: ['confirmed', 'plausible', 'refuted'] },
    severity: { type: 'string', enum: SEV },
    category: { type: 'string', enum: CATS },
    reason: { type: 'string', description: 'the evidence for the verdict; for refuted, exactly what the finding got wrong' },
    correction: { type: 'string', description: 'corrections to lines, mechanism, impact or fix; empty string if none' },
  },
  required: ['id', 'verdict', 'severity', 'category', 'reason', 'correction'],
}
const VERDICTS = { type: 'object', properties: { verdicts: { type: 'array', items: VERDICT } }, required: ['verdicts'] }
const DEDUP = {
  type: 'object',
  properties: {
    groups: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          keep: { type: 'string', description: 'id of the finding to keep; must be from your shard' },
          dupes: { type: 'array', items: { type: 'string' }, description: 'ids that share its root cause (may include ids from other shards)' },
          note: { type: 'string', description: 'the shared root cause in one sentence, and what you checked in the code to confirm it' },
          severity_note: { type: 'string', description: 'if members disagree on severity, which rating is right and why; empty otherwise' },
        },
        required: ['keep', 'dupes', 'note', 'severity_note'],
      },
    },
    near_misses: { type: 'array', items: { type: 'string' }, description: 'pairs that look alike but are NOT duplicates, as "idA / idB: why distinct" (only the non-obvious ones)' },
    notes: { type: 'string', description: 'anything left unchecked' },
  },
  required: ['groups', 'near_misses', 'notes'],
}
const CANDS = {
  type: 'object',
  properties: {
    uncovered: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          key: { type: 'string', description: 'short unique slug' },
          source_unit: { type: 'string' },
          file: { type: 'string' },
          line: { type: 'integer', description: '0 if unknown' },
          claim: { type: 'string', description: 'the concrete issue as the auditor described it, plus anything you confirmed by a quick look' },
          searched: { type: 'string', description: 'how you searched index.tsv to establish it is not already reported' },
        },
        required: ['key', 'source_unit', 'file', 'line', 'claim', 'searched'],
      },
    },
    covered: { type: 'array', items: { type: 'object', properties: { claim: { type: 'string' }, by: { type: 'array', items: { type: 'string' } } }, required: ['claim', 'by'] } },
    discarded: { type: 'array', items: { type: 'string' }, description: 'observations that are not concrete issues (ruled-out checks, vague remarks), one short line each' },
  },
  required: ['uncovered', 'covered', 'discarded'],
}

const CONTEXT = `Context: RelayV4 is a Rust workspace: a local engine (crates/relay-core: one SQLite store behind one mutex, a typed command bus over a Unix socket, PTY-backed AI-agent sessions in git worktrees, guardrails that gate what agents may run/write, Android device and emulator tools, scrcpy mirroring), the bus types (crates/relay-bus), the CLI and MCP servers (crates/relay-cli, including Unreal/Blender tooling that runs embedded Python), the phone door (crates/relay-remote), and a GTK4 desktop client (apps/relay-native). The phone app (apps/relay-mobile) is out of scope.
Round 1 of its first code audit ran at commit 361c6f9 and produced 1,103 verified findings. The code you read is the repository at ${ROOT}, which is slightly newer: since 361c6f9 these files changed (PR #30 and #31): ${'$'}(cat ${SCRATCH}/r2/changed_files.txt). Line numbers in those files may have drifted.
Round-1 data: ${AUD}/round1.json (full findings: results[].findings[] with id, title, file, line, evidence, impact, fix, related, v1, v2, status, final_severity). ${AUD}/index.tsv: one line per kept finding (id, severity, category, location, title), sorted by file; grep it.

HARD RULES
- Strictly read-only in the repository. Do not modify, create, delete, stage or commit anything there. Do not run cargo, rustc, npm or the app. Do not call any mcp__relay__* tool: they act on the user's live Relay engine.
- Use Read, Grep, Glob and read-only shell (rg, git log/show/blame, jq, python3 -I). Throwaway files go under ${SCRATCH}/exp/ only.`
const CTX = CONTEXT.replace('${'.concat("'$'", '}(cat ', SCRATCH, '/r2/changed_files.txt)'), 'listed in ' + SCRATCH + '/r2/changed_files.txt')

const RUBRIC = `SEVERITY (real-world impact: a single-user desktop app with a local engine, many concurrent agent sessions, and an optional phone door reachable over LAN or a rendezvous server)
- critical: data loss/corruption; a security hole reachable by another local user, a network peer, or an agent escaping its guardrails; an engine crash/hang that takes down every session; a frequent hard failure of a core flow.
- high: likely user-visible wrong behavior or failure in a normal flow; a freeze of the bus or the UI; unbounded growth in normal use; a safety check that can be silently bypassed.
- medium: a real bug in a less common path or edge case; a design/maintainability problem with a concrete ongoing cost (e.g. duplicated logic that has already diverged).
- low: a minor bug in a rare edge case; a clear simplification or cleanup with modest payoff.
CATEGORIES: bug, security, concurrency, invariant (breaks a CLAUDE.md rule: slow work in a handler registered with Engine::register while holding the store mutex, a subprocess not run through proc::output_with_timeout, session.input touching SQLite, migration rules), performance, resource-leak, error-handling, data-loss, dead-code, duplication, overengineering, code-quality, test-quality, docs-drift.
EVIDENCE STANDARD: cite a repo-relative file and 1-indexed line and quote the decisive code; trace callers/callees to show it is reachable and not handled elsewhere; give a concrete trigger -> consequence. The code is deliberately not rustfmt-clean: never report formatting, naming style or pedantic-lint trivia.`

function dedupPrompt(s) {
  return `${CTX}

YOUR ROLE: deduplicate one shard of round-1 findings. Round 1 ran 41 area auditors and 18 cross-cutting lens agents over the same code, so the same defect was often reported several times, sometimes at different call sites.
Your shard: ${IN}/shard-${s}.jsonl (one compact finding per line: id, sev, cat, status, skeptic (true if a skeptic verified it), loc, symbol, title, related, evidence and fix truncated). Read all of it. For full text of any finding: jq or grep round1.json by id.

DEFINITION. Two findings are duplicates when they share a root cause, so that one fix resolves both, even if they cite different call sites (example: "task.dispatch runs nested staged prepares in its transaction" cited at engine.rs:328 and at task.rs:683 is one finding). A shared theme is NOT enough: two different handlers that each run a subprocess under the lock are two findings, because each needs its own fix. Same file:line is NOT proof either: two different bugs can sit on one line. When unsure, open the code and decide; when still unsure, do not merge.

METHOD
1. Group your shard's findings that are duplicates of each other.
2. For every finding in your shard, grep ${AUD}/index.tsv (by file, symbol, op name, key words of the title) for duplicates OUTSIDE your shard, especially from lens agents whose primary file is elsewhere. Include those ids in "dupes" when they are clearly the same root cause. Do not create groups whose keep is outside your shard.
3. Choose "keep": the one a skeptic verified (skeptic=true) if any, then the most accurate and complete one. Prefer keep from your shard; if the best member is outside your shard, still put your shard's best member as keep.
4. Return only groups with at least one dupe. In "note", state the shared root cause and what you checked. In "severity_note", if the members' severities differ, say which is right per the rubric and why.

${RUBRIC}

Return via StructuredOutput.`
}

function minePrompt(i) {
  return `${CTX}

YOUR ROLE: completeness pass. Round-1 auditors each wrote a coverage note (what they read, what they ruled out, and observations outside their chunk they did NOT report). Your half of those notes: ${IN}/coverage-${i}.md. Read all of it.
Pull out every CONCRETE issue an auditor noticed but did not report (typically "out of chunk", "not reported", "noted", "also saw", "outside scope", "worth checking"). Skip descriptions of what was ruled out as fine, and vague remarks with no concrete code issue.
For each concrete issue, grep ${AUD}/index.tsv (file, symbol, op name, key title words) and, if needed, round1.json, to decide whether an existing finding already covers it (same root cause). Covered -> "covered" with the ids. Not covered -> "uncovered" with the file, line (0 if unknown) and the claim; take a quick look at the code to give the right file and line, but do not write a full finding.
Candidates already known to be uncovered (include them if your half mentions them): socket.rs reads request lines with unbounded BufReader::lines(); MirrorRuntime::send_control does a blocking write_all (device.rs:279-283); workspace.discover walks a tree to depth 4 inside a locked handler (workspace.rs:115-133, 393-413); attach_bytes writes files before the transaction commits (task.rs:505-530).

Return via StructuredOutput.`
}

function writeupPrompt(batch) {
  return `${CTX}

YOUR ROLE: auditor. Each candidate below is an issue a round-1 auditor noticed in passing but never reported. For each, read the code and either write it up as a full finding meeting the evidence standard, or drop it with the reason (not real, unreachable, already handled, already covered by an index.tsv finding, or not worth reporting). Check index.tsv once more for an existing finding with the same root cause before writing it up.

${RUBRIC}

CANDIDATES
${batch.map(c => JSON.stringify(c)).join('\n\n')}

Return via StructuredOutput, using each candidate's "key" as "cand".`
}

function verifyPrompt(batch) {
  return `${CTX}

${RUBRIC}

YOUR ROLE: adversarial verifier. Below are ${batch.length} candidate findings from another auditor. Auditors over-report; kill what is wrong and calibrate what is right. For EACH finding:
1. Open the cited code yourself and read enough surrounding code and callers to judge independently. Do not trust the finding's quotes or line numbers.
2. Try to refute it: does the code really do what is claimed? Is the trigger reachable (callers, guards, validation, transactions, locks upstream)? Is it handled elsewhere? Is it an intended, documented, harmless decision (docs/engine/DECISIONS.md, BUS.md)? For quality findings: is the problem real, and would the proposed change genuinely be simpler without losing something the code needs?
3. Verdict: "confirmed" (you independently established it, possibly with corrections), "plausible" (probably real, but the trigger or impact could not be fully established), or "refuted" (wrong, unreachable, already handled, or not worth changing). Between plausible and refuted, choose refuted.
4. Re-rate severity with the rubric, calibrated to real impact in this app, and fix the category if wrong. Put corrections in "correction".

FINDINGS
${batch.map(f => JSON.stringify(f)).join('\n\n')}

Return exactly one verdict per finding id via StructuredOutput.`
}

function chunk(arr, n) {
  const out = []
  for (let i = 0; i < arr.length; i += n) out.push(arr.slice(i, i + n))
  return out
}

const dedupP = parallel(SHARDS.map(s => async () => {
  const r = await agent(dedupPrompt(s), { label: `dedup:${s}`, phase: 'Dedup', schema: DEDUP, effort: 'high' })
  return r ? { shard: s, ...r } : null
}))

const mineP = pipeline(
  [1, 2],
  i => agent(minePrompt(i), { label: `mine:coverage-${i}`, phase: 'Mine', schema: CANDS, effort: 'high' }),
  async (cands, i) => {
    if (!cands) return null
    const unc = (cands.uncovered || []).map((c, k) => ({ ...c, key: `cov${i}-${k + 1}-${c.key}` }))
    unc.sort((a, b) => (a.file < b.file ? -1 : a.file > b.file ? 1 : a.line - b.line))
    log(`coverage-${i}: ${unc.length} uncovered, ${(cands.covered || []).length} already covered`)
    const batches = chunk(unc, 5)
    const done = await parallel(batches.map((b, bi) => async () => {
      const w = await agent(writeupPrompt(b), { label: `writeup:cov${i}:${bi + 1}`, phase: 'Write-up', schema: WRITEUP, effort: 'high' })
      if (!w) return { findings: [], dropped: b.map(c => ({ cand: c.key, why: 'write-up agent failed' })) }
      const fs = (w.findings || []).map((x, k) => ({ ...x.finding, id: `r2-cov${i}-${bi + 1}#${k + 1}`, cand: x.cand, source: 'r2-completeness' }))
      const vmap = {}
      let pending = fs
      for (let attempt = 0; attempt < 2 && pending.length; attempt++) {
        const v = await agent(verifyPrompt(pending), { label: `verify:cov${i}:${bi + 1}${attempt ? 'r' : ''}`, phase: 'Verify', schema: VERDICTS, effort: 'high' })
        if (v && v.verdicts) v.verdicts.forEach(x => { if (pending.some(p => p.id === x.id)) vmap[x.id] = x })
        pending = pending.filter(p => !vmap[p.id])
      }
      return { findings: fs.map(f => ({ ...f, v1: vmap[f.id] || null })), dropped: w.dropped || [] }
    }))
    return {
      miner: i, candidates: unc, covered: cands.covered || [], discarded: cands.discarded || [],
      findings: done.filter(Boolean).flatMap(d => d.findings), dropped: done.filter(Boolean).flatMap(d => d.dropped),
    }
  },
)

const [dedup, mined] = await Promise.all([dedupP, mineP])
const groups = dedup.filter(Boolean).reduce((n, d) => n + d.groups.length, 0)
log(`dedup: ${dedup.filter(Boolean).length}/${SHARDS.length} shards, ${groups} groups`)
return { dedup, mined }
