#!/usr/bin/env node
// bar-score.mjs — advisory noise-bar feedback for memory proposals (issue #420).
//
// The memory protocol's bar (durable, non-obvious, not recoverable elsewhere) was prose;
// the validator could only check shape. This module scores a proposal batch against the
// bar's three dimensions in ONE judgment call and returns a line of feedback.
//
// It is advisory BY DESIGN and cannot do anything else: it returns a string. It never
// throws, never blocks, never edits the records — the protocol measured what happens
// when a gate sits on this path (the retracted recall-and-HOLD gate, four defects) and
// the lesson is written there: advice at the producer, dumb pipe everywhere else.

import { createClient, scoreQuestion } from './typesafe.mjs'

const QUESTIONS = {
  durable: scoreQuestion(
    'Rate each memory proposal: will this finding still matter and still be true when read in a year?',
    [
      'ephemeral: about this run or session only',
      'soon stale: tied to current state that will change',
      'moderately durable: partly tied to current state',
      'durable: outlives the code state it describes',
      'permanently useful: true and valuable indefinitely',
    ]
  ),
  non_obvious: scoreQuestion(
    'Rate each memory proposal: how hard would it be for a competent reader to re-derive this from the diff, the PR, or the tracker alone?',
    [
      'trivially re-derivable: it restates the diff or PR body',
      'mostly a restatement with slight framing added',
      'partly non-obvious: some framing is new',
      'non-obvious: the why or the trap is not in the artifacts',
      'impossible to re-derive: lived experience only',
    ]
  ),
  self_contained: scoreQuestion(
    'Rate each memory proposal: is this ONE concept, readable standalone without the session that produced it?',
    [
      'needs the conversation to parse',
      'several concepts tangled together',
      'mostly self-contained',
      'one concept, self-contained',
      'one concept, self-contained, and precisely worded',
    ]
  ),
}

/**
 * @param {Array<{concept: string, content: string, summary?: string}>} records
 * @param {{ask: Function}} [client] a TypeSafe client or test double; omit to build one.
 * @returns {Promise<string>} one feedback line for the proposing session. Never throws.
 */
export async function scoreBar(records, client) {
  if (!records?.length) return ''
  const c = client || createClient()
  // Only the fields the bar judges travel as state — vault/importance are routing
  // metadata, not findings, and stay out of the judgment.
  const state = records.map((r) => ({ concept: r.concept, content: r.content, summary: r.summary }))

  let r
  try {
    r = await c.ask({ state, questions: QUESTIONS })
  } catch {
    return 'memory-propose: bar scoring skipped (typesafe: client-error) — append proceeds.'
  }
  if (!r.ok) return `memory-propose: bar scoring skipped (typesafe: ${r.reason}) — append proceeds.`

  // F8: a score is on its level scale (0..criteria.length-1) or it is not an answer.
  const num = (a, id) => {
    const s = a?.score
    const max = QUESTIONS[id].criteria.length - 1
    return typeof s === 'number' && Number.isFinite(s) && s >= 0 && s <= max ? s : null
  }
  const d = num(r.answers.durable, 'durable')
  const n = num(r.answers.non_obvious, 'non_obvious')
  const s = num(r.answers.self_contained, 'self_contained')
  if (d === null || n === null || s === null) {
    return 'memory-propose: bar scoring skipped (typesafe: invalid-answer) — append proceeds.'
  }

  const line = `memory-propose: bar feedback (advisory, never blocks): durable ${d.toFixed(1)}/4, non-obvious ${n.toFixed(1)}/4, self-contained ${s.toFixed(1)}/4`
  const composite = (d + n + s) / 12
  if (composite < 0.6) {
    return `${line} — composite ${composite.toFixed(2)} reads below the bar; the do-not-propose list is in .claude/memory-protocol.md (your call; nothing is blocked).`
  }
  return line
}
