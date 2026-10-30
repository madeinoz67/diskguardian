# .claude/hooks — drain internals

> Moved verbatim from `.claude/memory-protocol.md` (BPE trim, 2026-10-15): the drain
> contract properties and the retracted HELD-gate postmortem. The protocol keeps the
> generalisable lesson and points here.

## Drain internals

The drain's contract, in the order the properties matter:

1. **Nothing is lost, including concurrently.** A proposal that cannot be written *this
   time* stays in the ledger; one that can *never* be written moves to
   `memory-proposals.deadletter.jsonl` with the reason, so the queue can reach empty; a
   proposal appended *during* a run survives it. The drain consumes only the byte range it
   read, splices anything appended past that offset back in verbatim, and refuses to
   rewrite at all if the region it consumed changed underneath it.

   The consumed range stops at the **last newline**, never at the file size: an
   unterminated trailing line is a writer mid-write, and dead-lettering it would rule a
   transient state permanently invalid. It stays in the tail until the rest of it arrives.

   A **producer never fails on a held lock** — it appends anyway and says so on stderr. An
   unlocked append lands past the offset the drain pinned, which is the tail the splice
   preserves; refusing to append would convert a held lock into a lost finding, which is the
   one failure this whole mechanism exists to prevent.
2. **Idempotent, exactly.** Every proposal carries a content-derived `op_id` and the server
   holds an idempotency receipt for it, so a re-drain returns the existing engram
   (`idempotent: true`) rather than minting a second. That is an O(1) exact check with no
   embedder — as opposed to asking a similarity score whether two things are the same
   thing. (Receipts expire after 30 days, which is far longer than a line can survive in the
   ledger now that the drain is wired.)

   **Identity is `vault` + `concept` + `content`, and nothing else.** `summary`, `type`,
   `entities`, `tags` and `importance` are annotations on that identity: they are written on
   a first write, and a re-proposal that *corrects* one of them does **not** apply it,
   because the op_id is unchanged and the server returns the existing engram untouched.
   Including them in identity would mint a rival near-duplicate for a one-word summary edit,
   which is exactly the pollution this design exists to avoid — so the drain instead says so
   out loud: a `NOT APPLIED` line, `counts.unapplied_annotations` in the receipt, and
   `annotations_not_applied` on the archive record, which also keeps the corrected text.
   **Correcting a live memory is `muninn_evolve`'s job, not a re-proposal's.**
3. **Observable from outside.** *Every* invocation writes `memory-drain-receipt.json` and
   appends to `memory-drain-receipts.jsonl` — including the no-op, debounced, locked and
   failure paths — carrying timestamp, trigger, items considered, items acted on, and
   per-outcome counts. Before this, the archive was appended only on success, so "never
   invoked", "invoked, ledger empty" and "invoked, everything held" were the identical
   filesystem state: no file. Three sessions could only answer "has this ever run?" by
   inference.

   A receipt nobody reads is not observability, though — it is a file. So
   `memory-freshness.mjs` runs at `SessionStart`, reads the receipt and the queue, and says
   something **only when something is wrong**: an unhealthy last outcome, proposals still
   queued, or a ledger with no receipt at all. It never blocks and always exits 0. Without
   it the receipt was write-only, and a seconds-old receipt reading `debounced` while
   findings rot looks like an answer rather than the absence of one.

   Two bounds keep the receipt honest under failure: every daemon call has a per-request
   timeout (`--timeout`, default 10 s) and the run has a deadline (`--deadline`, default
   45 s, under the hooks' 60 s kill), and `SIGTERM`/`SIGINT`/`SIGHUP` release the lock and
   write an `interrupted` receipt. Before that, a daemon that accepted the connection and
   never answered hung the drain until the harness killed it — leaving the lock on disk and
   **no** receipt, so `stat` returned the previous one, fresh-looking and wrong, and the
   leaked lock blocked producers for the ~10 minutes until the stale breaker fired.
   `SIGKILL` remains unrecoverable by construction; the stale-lock breaker is what covers it.

   The receipt's `last_run_at` — the debounce watermark — advances **only** on an outcome
   that actually consumed from the ledger (`ok`, `partial`, `empty`, `no-ledger`,
   `rewrite-refused`). It does not advance on `locked`, `unreachable`, `error`,
   `interrupted` or `debounced`. It only ever moves forward and an untouched ledger's mtime
   does not move at all, so one failed run that advanced it disabled the debounced `Stop`
   trigger *permanently* — for exactly the crash/kill case `Stop` exists to cover.
4. **Quiet in a hook, loud by hand.** Invoked as a hook it always exits 0 and the receipt
   carries the truth — a hook that reports failure at every session close is a hook people
   disable, which is the failure this mechanism exists to prevent. Invoked manually it exits
   non-zero when a write genuinely failed.

### What the drain deliberately no longer does

It used to recall the neighbourhood before each write and **HOLD** anything with a
strong-banded match, so a script would never mint a rival copy. That claim is retracted: the
gate never executed once in its lifetime, and four defects lived in it —

- it cost ~0.9 s/proposal, which is what made the drain unusable in a hook;
- it was structurally dead on any vault whose fusion mode is `rrf` or `weighted_sum`, and
  whenever semantic search is degraded, because the engine then bands every row
  `uncalibrated` and `strong` is never emitted. The drain wrote everything while reporting
  "0 held" — indistinguishable from "0 duplicates found". (Generalisable: any consumer
  branching on `relevance_band` must read `uncalibrated` and `filter_match` as *absence of
  measurement*, never as a low band.)
- a kill mid-loop left engrams written and the ledger unchanged, so on the next run each
  proposal recalled its own freshly-written self, banded strong, and was HELD **forever**;
- HELD was unbounded and grew *because* the drain succeeded, since every write densifies the
  neighbourhood the next proposal is tested against.

The generalisable shape: **a gate whose discriminating variable moves at a rate correlated
with the phenomenon it suppresses fails anti-correlated with need.**
