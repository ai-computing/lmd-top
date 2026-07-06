export const meta = {
  name: 'auto-improve',
  description: 'Autonomous plan->implement->review->commit loop for lmd-top: Fable discovers & plans one improvement, Opus implements + cargo check, Fable reviews the diff and auto-commits on pass (reverts on fail).',
  whenToUse: 'Autonomous project maintenance: find and land small high-value improvements to the lmd-top codebase without human intervention. Pass {iterations: N} to control cycle count (default 3).',
  phases: [
    { title: 'Plan', detail: 'Fable scans the repo and picks ONE concrete, low-risk improvement', model: 'fable' },
    { title: 'Implement', detail: 'Opus implements the plan and runs cargo check', model: 'opus' },
    { title: 'Review', detail: 'Fable reviews the diff; auto-commit on pass, revert on fail', model: 'fable' },
  ],
}

const ITERATIONS = (args && args.iterations) || 3

const PLAN_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['skip', 'title', 'category', 'rationale', 'files', 'steps'],
  properties: {
    skip: { type: 'boolean', description: 'true if nothing worthwhile remains to do' },
    title: { type: 'string', description: 'imperative one-line summary of the improvement' },
    category: { type: 'string', enum: ['bugfix', 'cleanup', 'refactor', 'docs', 'feature', 'perf', 'test'] },
    rationale: { type: 'string', description: 'why this is worth doing and why it is low-risk' },
    files: { type: 'array', items: { type: 'string' }, description: 'repo-relative files expected to change' },
    steps: { type: 'array', items: { type: 'string' }, description: 'concrete implementation steps' },
  },
}

const IMPL_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['files_changed', 'check_passed', 'check_output', 'summary'],
  properties: {
    files_changed: { type: 'array', items: { type: 'string' } },
    check_passed: { type: 'boolean', description: 'true only if `cargo check` exited 0' },
    check_output: { type: 'string', description: 'tail of cargo check output (last ~15 lines)' },
    summary: { type: 'string', description: 'what was actually changed' },
  },
}

const REVIEW_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['verdict', 'reasons', 'commit_message'],
  properties: {
    verdict: { type: 'string', enum: ['pass', 'fail'] },
    reasons: { type: 'string', description: 'concise justification for the verdict' },
    commit_message: { type: 'string', description: 'conventional-commit message to use if verdict is pass' },
  },
}

function planPrompt(done, i) {
  return [
    'You are the PLANNER (Fable) for an autonomous improvement loop on the lmd-top Rust TUI project.',
    'The working tree is on branch `auto-improve` and is CLEAN (all prior work committed).',
    '',
    'Task: scan the codebase and pick exactly ONE concrete, LOW-RISK, high-value improvement to make this iteration.',
    'Prefer: real bug fixes, dead-code / clippy-style cleanups, error-handling gaps, small correctness or clarity wins.',
    'AVOID: large refactors, risky behavior changes, anything needing a live cluster to validate, and cosmetic churn.',
    'The change MUST be verifiable by `cargo check` alone (no runtime cluster access).',
    '',
    'Constraints on your investigation: use Read/Grep/Bash (e.g. `cargo clippy` if available, `git log`, `rg`).',
    'Keep the change small enough that Opus can implement it and it compiles cleanly.',
    '',
    done.length ? `Already completed this session (do NOT repeat or undo these):\n- ${done.join('\n- ')}` : 'Nothing completed yet this session.',
    '',
    `This is iteration ${i + 1} of ${ITERATIONS}. If you genuinely cannot find a worthwhile low-risk improvement, set skip=true.`,
    'Return the structured plan.',
  ].join('\n')
}

function implPrompt(plan) {
  return [
    'You are the IMPLEMENTER (Opus) for an autonomous improvement loop on the lmd-top Rust project.',
    'The working tree is CLEAN on branch `auto-improve`. Implement EXACTLY the plan below and nothing else.',
    '',
    `Title: ${plan.title}`,
    `Category: ${plan.category}`,
    `Rationale: ${plan.rationale}`,
    `Expected files: ${plan.files.join(', ')}`,
    'Steps:',
    ...plan.steps.map((s, k) => `  ${k + 1}. ${s}`),
    '',
    'Rules:',
    '- Make focused edits with Edit/Write. Match surrounding code style (comments in Korean where the file uses Korean).',
    '- Do NOT touch files unrelated to the plan. Do NOT run git commit or git add.',
    '- After editing, run `cargo check --message-format=short` and capture the result.',
    '- If it fails, fix your own changes until it compiles, or revert them if the plan turns out unworkable.',
    'Report which files you changed, whether cargo check passed, the tail of its output, and a summary.',
  ].join('\n')
}

function reviewPrompt(plan, impl) {
  return [
    'You are the REVIEWER (Fable) for an autonomous improvement loop on the lmd-top Rust project.',
    'Review the uncommitted diff against the plan. Be strict but fair.',
    '',
    `Plan title: ${plan.title}`,
    `Plan steps: ${plan.steps.join(' | ')}`,
    `Implementer summary: ${impl ? impl.summary : '(implementer produced no result)'}`,
    `Implementer reported cargo check passed: ${impl ? impl.check_passed : false}`,
    '',
    'Do this:',
    '1. Run `git diff` to see the actual changes.',
    '2. Independently run `cargo check --message-format=short` to confirm it compiles (exit 0).',
    '3. Judge: does the diff correctly and safely implement the plan? No regressions, no scope creep, no debug leftovers, no unrelated files touched?',
    '',
    'Verdict FAIL if: cargo check fails, the diff is empty, it strays from the plan, it introduces a plausible bug, or it touches unrelated files.',
    'Verdict PASS only if the change is correct, in-scope, and compiles.',
    'If PASS, provide a conventional-commit message (e.g. "fix(kube): ...") describing the change.',
    'Return the structured verdict.',
  ].join('\n')
}

const results = []
const done = []

for (let i = 0; i < ITERATIONS; i++) {
  phase('Plan')
  const plan = await agent(planPrompt(done, i), {
    label: `plan#${i + 1}`, phase: 'Plan', model: 'fable', effort: 'high',
    agentType: 'general-purpose', schema: PLAN_SCHEMA,
  })

  if (!plan || plan.skip) {
    log(`iter ${i + 1}: planner found nothing worthwhile — stopping loop`)
    break
  }
  log(`iter ${i + 1} PLAN [${plan.category}]: ${plan.title}`)

  phase('Implement')
  const impl = await agent(implPrompt(plan), {
    label: `impl#${i + 1}`, phase: 'Implement', model: 'opus', effort: 'high',
    agentType: 'general-purpose', schema: IMPL_SCHEMA,
  })

  phase('Review')
  const review = await agent(reviewPrompt(plan, impl), {
    label: `review#${i + 1}`, phase: 'Review', model: 'fable', effort: 'high',
    agentType: 'general-purpose', schema: REVIEW_SCHEMA,
  })

  const ok = review && review.verdict === 'pass' && impl && impl.check_passed
  if (ok) {
    const msg = (review.commit_message || plan.title).replace(/"/g, '\\"')
    await agent(
      `Commit the current changes on branch auto-improve. Run exactly:\n` +
      `git add -A && git commit -q -m "${msg}" -m "Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"\n` +
      `Then run \`git log --oneline -1\` and report the resulting commit line. Do not make any other changes.`,
      { label: `commit#${i + 1}`, phase: 'Review', model: 'fable', effort: 'low', agentType: 'general-purpose' },
    )
    done.push(plan.title)
    results.push({ iter: i + 1, title: plan.title, category: plan.category, status: 'committed', note: review.reasons })
    log(`iter ${i + 1}: ✅ COMMITTED — ${plan.title}`)
  } else {
    await agent(
      'Restore the working tree to the last commit. Run exactly: `git reset --hard HEAD && git clean -fd`. Then report `git status --short`. Do nothing else.',
      { label: `revert#${i + 1}`, phase: 'Review', model: 'fable', effort: 'low', agentType: 'general-purpose' },
    )
    results.push({
      iter: i + 1, title: plan.title, category: plan.category, status: 'reverted',
      note: review ? review.reasons : 'no review result', check_passed: impl ? impl.check_passed : false,
    })
    log(`iter ${i + 1}: ↩️ REVERTED — ${review ? review.verdict : 'no-review'}`)
  }
}

return { iterations_run: results.length, committed: results.filter(r => r.status === 'committed').length, results }
