# Memory proposals — how a finding survives the session

A session produces knowledge; the little of it that is not recoverable from git, the PR, or
the issue is what memory is for — persistence that depends on someone remembering to
remember does not happen under load. So: **do not print a memory. Append it to the ledger.**
The ledger is a file, and files do not evaporate.

- Ledger: `.claude/memory-proposals.jsonl` (gitignored, append-only)
- Producer: `node .claude/hooks/memory-propose.mjs` (validates, fills `vault`)
- Vault: **diskguardian** (MuninnDB, `http://127.0.0.1:8125/mcp`)
- Drain: `.claude/hooks/memory-drain.mjs`, wired to PreCompact / SessionEnd / Stop

## The bar — what qualifies

A proposal must be **durable, non-obvious, and not recoverable elsewhere**: a measured
result and its number, a decision and why it beat the alternative, an honest negative with
the evidence that killed it, a defect *pattern*, context a future session would otherwise
rediscover, or a trap that looks safe.

Do not propose progress narration, restatements of a diff or commit, anything you would
look up again anyway, or five variations of one idea. One atomic concept per memory. A
noisy vault is worse than a small one.

**One class is exempt from the noise bar: review findings.** Every review — code-reviewer,
adversary, mechanism-critic, or a human review relayed by an agent — appends its findings
record before the session ends, at **every severity including LOW and NIT**, each finding
with its disposition (fixed / deferred / accepted-risk).

## How to append

**Recall before you propose.** Spend one `muninn_recall` on the vault using the fact's own
concepts. If it already holds the fact, `muninn_evolve` it or cite it — never mint a rival
copy. That judgment belongs to the proposing session, never to a hook.

Every proposal carries **at least one tag**; the validator rejects tag-less proposals.

Write it self-contained: a memory that only makes sense next to its conversation is a
comment, not a memory.

## How it drains

`PreCompact`, `SessionEnd`, and a debounced `Stop` are wired in `.claude/settings.json`. To
run it by hand: `node .claude/hooks/memory-drain.mjs` (`--dry-run`, `--max N`).

**The drain is a pipe, not a curator** — identity → write → archive → truncate; no recall,
no relevance computation. Identity is exact: every proposal carries a content-derived
`op_id`, and the server's idempotency receipt means a re-drain returns the existing engram
rather than a duplicate.

The drain credential is `~/.muninn/flush-keys/diskguardian.token`. A proposal whose vault
has no credential stays queued, and the receipt says so.

### Curation — deferred, not dropped

Supersession curation is a human-run pass over the vault (adjudicated with
`muninn_evolve`). `memory-proposals.drained.jsonl` records every engram id the drain
produced, which is exactly the candidate set such a pass needs.

### Worktrees — one ledger per repository

There is exactly one ledger per repository: the main checkout's. `MUNINN_LEDGER_ROOT`
overrides it.

## Privacy

No vault names, no client or tenant identifiers, no memory content copied out of someone
else's vault, no commercial terms. The ledger is gitignored; that is a backstop, not
permission.
