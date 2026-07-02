export const meta = {
  name: 'bug-security-perf-hunter',
  description: 'Multi-lens bug/security/performance audit with adversarial verification and optional auto-fix PR',
  whenToUse:
    'Run before a PR, after a feature lands, or periodically to sweep for correctness bugs, security vulnerabilities, and performance issues. Pass args to control target/behavior: {scope: "diff"|"full"|"<path>", base: "master", fix: false, voters: 3}. fix defaults to false (report-only) — set fix:true to have it auto-fix confirmed findings in an isolated worktree, commit, push, and open a PR.',
  phases: [
    { title: 'Find', detail: 'fan out across bug/concurrency/security/performance lenses' },
    { title: 'Verify', detail: 'adversarial multi-vote refutation per finding' },
    { title: 'Fix', detail: 'only runs if args.fix === true: isolated worktree, commit, push, PR' },
  ],
}

const scope = (args && args.scope) || 'diff'
const base = (args && args.base) || 'master'
const doFix = !!(args && args.fix === true)
const voters = (args && args.voters) || 3

const FINDING_SCHEMA = {
  type: 'object',
  properties: {
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          title: { type: 'string' },
          category: { type: 'string', enum: ['bug', 'security', 'performance'] },
          severity: { type: 'string', enum: ['low', 'medium', 'high', 'critical'] },
          file: { type: 'string' },
          line: { type: 'number' },
          description: { type: 'string' },
          failure_scenario: { type: 'string' },
        },
        required: ['title', 'category', 'severity', 'file', 'description', 'failure_scenario'],
      },
    },
  },
  required: ['findings'],
}

const VERDICT_SCHEMA = {
  type: 'object',
  properties: {
    refuted: { type: 'boolean' },
    reason: { type: 'string' },
  },
  required: ['refuted', 'reason'],
}

let targetDescription

if (scope === 'full') {
  targetDescription = 'the entire codebase (all crates in this workspace)'
} else if (scope === 'diff') {
  targetDescription = `the diff between the current branch and ${base} (run: git diff ${base}...HEAD, plus git status for uncommitted changes) — but read full surrounding files/callers as needed, not just the diff hunks in isolation`
} else {
  targetDescription = `the path ${scope}`
}

log(`Target: ${targetDescription} | fix mode: ${doFix ? 'ON (will open a PR)' : 'off (report-only)'}`)

const LENSES = [
  {
    key: 'bug-correctness',
    category: 'bug',
    prompt:
      `You are a correctness bug hunter reviewing ${targetDescription} in a Rust workspace. Focus on: logic errors, ` +
      `off-by-one, incorrect error propagation, panics (unwrap/expect/indexing) on unvalidated input, use of stale/moved ` +
      `state, integer overflow, and broken invariants. Read the actual code, not just isolated diff hunks — check callers ` +
      `and callees where needed. Report every concrete, reproducible bug with file, line, and a specific failure scenario ` +
      `(exact inputs/state that trigger it). Do not report style nits or hypothetical issues without a concrete trigger.`,
  },
  {
    key: 'bug-concurrency',
    category: 'bug',
    prompt:
      `You are a concurrency/async bug hunter reviewing ${targetDescription} in a Rust workspace (tokio/axum). Focus on: ` +
      `deadlocks, lock ordering issues, holding locks across .await, channel misuse (unbounded growth, deadlock on ` +
      `full/closed channel), task cancellation leaving state inconsistent, shared mutable state without proper ` +
      `synchronization, and Arc/ArcSwap-backed hot-reload primitives read/written inconsistently. Report every concrete ` +
      `issue with file, line, and the exact interleaving/timing that triggers it.`,
  },
  {
    key: 'security',
    category: 'security',
    prompt:
      `You are a security auditor reviewing ${targetDescription}. Focus on: injection (command/SQL/path traversal), ` +
      `unsafe deserialization of untrusted input, missing auth/authz checks on RPC or HTTP handlers, secrets in code or ` +
      `logs, unchecked unsafe{} blocks, any dependency drift toward rsa/openssl-family crypto crates (this repo explicitly ` +
      `avoids them for CI/security reasons — flag any new transitive dependency on them), TLS/cert validation bypass, and ` +
      `DoS vectors (unbounded allocation, unbounded recursion, algorithmic complexity attacks on attacker-controlled ` +
      `input). Report every concrete vulnerability with file, line, and the exact attacker-controlled input/steps to ` +
      `exploit it.`,
  },
  {
    key: 'performance',
    category: 'performance',
    prompt:
      `You are a performance auditor reviewing ${targetDescription} in a Rust workspace. Focus on: unnecessary clones or ` +
      `allocations in hot paths, O(n^2)+ algorithms where a better bound is feasible, lock contention (locks held too ` +
      `long or too broadly — prefer split ownership or channels over shared mutexes held across work), blocking calls ` +
      `inside async contexts, excessive Arc/Mutex churn, repeated serialization/deserialization, and N+1-style repeated ` +
      `work in loops that could be batched. Report every concrete issue with file, line, why it's slow, and the expected ` +
      `impact.`,
  },
]

phase('Find')

const findRounds = await pipeline(LENSES, (lens) =>
  agent(lens.prompt, { label: `find:${lens.key}`, phase: 'Find', schema: FINDING_SCHEMA }).then((r) =>
    r && r.findings ? r.findings.map((f) => ({ ...f, category: f.category || lens.category })) : [],
  ),
)

const allFindings = findRounds.filter(Boolean).flat()

log(`${allFindings.length} raw findings across ${LENSES.length} lenses`)

phase('Verify')

const verified = await pipeline(allFindings, (finding) =>
  parallel(
    Array.from(
      { length: voters },
      (_, i) => () =>
        agent(
          `Adversarially verify this reported ${finding.category} finding. Try hard to REFUTE it — read the actual ` +
            `file/lines and surrounding context yourself rather than trusting the report. If you cannot find a real, ` +
            `concrete way this triggers, mark refuted=true. Default to refuted=true if you are not fully convinced.\n\n` +
            `Title: ${finding.title}\nFile: ${finding.file}${finding.line ? ':' + finding.line : ''}\n` +
            `Description: ${finding.description}\nClaimed failure scenario: ${finding.failure_scenario}`,
          { label: `verify:${finding.file}`, phase: 'Verify', schema: VERDICT_SCHEMA },
        ),
    ),
  ).then((votes) => {
    const cast = votes.filter(Boolean)
    const refutations = cast.filter((v) => v.refuted).length
    const survives = cast.length > 0 && refutations * 2 < cast.length

    return { ...finding, survives, votes: cast }
  }),
)

const confirmed = verified.filter((f) => f.survives)
const bySeverityOrder = { critical: 0, high: 1, medium: 2, low: 3 }

confirmed.sort((a, b) => (bySeverityOrder[a.severity] ?? 9) - (bySeverityOrder[b.severity] ?? 9))

log(`${confirmed.length}/${allFindings.length} findings survived adversarial verification`)

if (!doFix || confirmed.length === 0) {
  return { scope, base, fixRequested: doFix, confirmed, totalRaw: allFindings.length }
}

phase('Fix')

const findingsBlock = confirmed
  .map(
    (f, i) =>
      `${i + 1}. [${f.severity}/${f.category}] ${f.title} — ${f.file}${f.line ? ':' + f.line : ''}\n` +
      `   ${f.description}\n   Trigger: ${f.failure_scenario}`,
  )
  .join('\n\n')

const fixReport = await agent(
  `You are fixing confirmed bug/security/performance findings in this Rust workspace, working in a fresh git worktree ` +
    `on a new branch.\n\nConfirmed findings to fix:\n\n${findingsBlock}\n\nInstructions:\n` +
    `- Create and check out a new branch named audit/hunter-fixes-<short-topic> off the current HEAD.\n` +
    `- Fix each finding with the minimal correct change. Follow existing code style (blank line after every block, ` +
    `variables grouped at the top of scope, isolated trailing return with a blank line above). Do not add comments ` +
    `except to document safety/invariants. Do not refactor beyond what each fix requires.\n` +
    `- After all fixes, build and run the relevant tests/clippy for the workspace and confirm they pass. Fix anything ` +
    `you break.\n` +
    `- Commit with 'mise exec -- git commit' (this repo's required convention for loading the correct git profile). Do ` +
    `not amend or force-push.\n` +
    `- Push the branch with 'mise exec -- git push -u origin <branch>'.\n` +
    `- Open a PR with 'gh pr create' based on ${base}, whose body lists each finding fixed (severity, file, one-line ` +
    `description) and a test plan.\n` +
    `- Report back: the branch name, PR URL, and which findings (if any) you could not safely fix and why.`,
  { label: 'fix-and-pr', phase: 'Fix', isolation: 'worktree' },
)

return { scope, base, fixRequested: doFix, confirmed, totalRaw: allFindings.length, fix: fixReport }
