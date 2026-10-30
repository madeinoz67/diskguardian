#!/usr/bin/env node
// typesafe.mjs — the one place this repo talks to the TypeSafe System One API.
//
// Contract (issue #420): every caller owns its fallback. ask() returns
//   { ok: true,  answers, model, usage }    — judge, then threshold in code
//   { ok: false, reason }                   — run your fallback
// with reason in: disabled | no-key | timeout | network | http-<status> | invalid-response.
//
// The unavailable path is a DESIGNED path, not an error path:
//   - TYPESAFE_DISABLE (any value that trims to non-empty) short-circuits before key
//     resolution — the deterministic lever for tests and for running the fallbacks on
//     purpose.
//   - One attempt, bounded timeout (default 10 s). No retry: retrying inside the client
//     multiplies the offline latency every caller pays, and the fallback IS the recovery.
//   - The await is raced against the timer in this module — a fetch implementation that
//     ignores the abort signal still cannot hang the caller.
//   - The API key never appears in any result, error, or thrown value.
//
// Key resolution order: explicit opts.key → TYPESAFE_API_KEY env → the operator's
// ~/.claude/.env (TYPESAFE_ENV_FILE overrides the location). Zero dependencies: node ≥ 18
// global fetch. API: POST {endpoint} with Authorization: Bearer only — never in the URL.

import { readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'

const DEFAULT_ENDPOINT = 'https://api.typesafe.ai/v1/systemone'
const DEFAULT_MODEL = 'jev-latest'
const DEFAULT_TIMEOUT_MS = 10_000

/** Load KEY=VALUE lines from a dotenv-style file. Never logs values. */
function readEnvFile(path) {
  let text
  try {
    text = readFileSync(path, 'utf8')
  } catch {
    return {}
  }
  const out = {}
  for (const line of text.split('\n')) {
    const m = line.match(/^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)\s*$/)
    if (m) out[m[1]] = m[2].replace(/^['"]|['"]$/g, '')
  }
  return out
}

function resolveKey(opts) {
  if (typeof opts.key === 'string' && opts.key.length > 0) return opts.key
  if (process.env.TYPESAFE_API_KEY) return process.env.TYPESAFE_API_KEY
  // Hermeticity guard (F4): a hook spawned from a node:test process inherits
  // NODE_TEST_CONTEXT; tests must never reach the operator's key file, whatever the
  // spawning test remembered to set. An explicit env key still works for live tests.
  // Residual, deliberately uncovered (M2, lane 2): NODE_TEST_CONTEXT is ABSENT when a
  // test file is run directly (node tests/x.test.mjs) — the guard cannot see that
  // route; test spawns pin TYPESAFE_ENV_FILE to a nonexistent path instead.
  if (process.env.NODE_TEST_CONTEXT) return ''
  const envFile = opts.envFile || process.env.TYPESAFE_ENV_FILE || join(homedir(), '.claude', '.env')
  return readEnvFile(envFile).TYPESAFE_API_KEY || ''
}

/**
 * @param {{key?: string, envFile?: string, disable?: boolean, timeoutMs?: number,
 *          endpoint?: string, model?: string, fetchImpl?: Function}} opts
 */
export function createClient(opts = {}) {
  const endpoint = opts.endpoint || process.env.TYPESAFE_ENDPOINT || DEFAULT_ENDPOINT
  const model = opts.model || process.env.TYPESAFE_MODEL || DEFAULT_MODEL
  const envTimeout = Number(process.env.TYPESAFE_TIMEOUT_MS)
  const timeoutMs = opts.timeoutMs ?? (Number.isFinite(envTimeout) ? envTimeout : DEFAULT_TIMEOUT_MS)
  const disabled = opts.disable ?? Boolean((process.env.TYPESAFE_DISABLE || '').trim().length > 0)
  const doFetch = opts.fetchImpl || fetch

  return {
    /** Ask a batch of independent questions over one state; they share a single call. */
    async ask({ state, questions }) {
      if (disabled) return { ok: false, reason: 'disabled' }
      const key = resolveKey(opts)
      if (!key) return { ok: false, reason: 'no-key' }

      const controller = new AbortController()
      const timer = setTimeout(() => controller.abort(), timeoutMs)
      // The race is owned here: even a fetch that ignores the signal cannot exceed the bound.
      const bounded = Symbol('typesafe-timeout')
      let raceTimer
      const timeoutPromise = new Promise((resolve) => {
        raceTimer = setTimeout(() => resolve(bounded), timeoutMs)
      })
      let res
      try {
        res = await Promise.race([
          doFetch(endpoint, {
            method: 'POST',
            headers: { Authorization: `Bearer ${key}`, 'Content-Type': 'application/json' },
            body: JSON.stringify({ state, model, questions }),
            signal: controller.signal,
          }),
          timeoutPromise,
        ])
      } catch (e) {
        // F7: a real fetch's abort can surface as an AbortError or as a wrapped cause;
        // the signal's own state is the authoritative discriminator.
        const timedOut = (e && e.name === 'AbortError') || controller.signal.aborted
        return { ok: false, reason: timedOut ? 'timeout' : 'network' }
      } finally {
        clearTimeout(timer)
        clearTimeout(raceTimer) // F3: an orphaned race timer held every caller's event loop open for the full timeout
      }
      if (res === bounded) return { ok: false, reason: 'timeout' }

      if (!res.ok) return { ok: false, reason: `http-${res.status}` }

      let body
      try {
        body = await res.json()
      } catch {
        return { ok: false, reason: 'invalid-response' }
      }
      if (!body || typeof body !== 'object' || !body.answers || typeof body.answers !== 'object') {
        return { ok: false, reason: 'invalid-response' }
      }
      return { ok: true, answers: body.answers, model: body.model, usage: body.usage }
    },
  }
}

/** Yes/no judgement; the answer carries the probability of yes. */
export function noulQuestion(instructions, criteria) {
  const q = { type: 'noul', instructions }
  if (criteria) q.criteria = criteria
  return q
}

/** One of a defined set; criteria maps option → rubric description. */
export function choiceQuestion(instructions, criteria) {
  return { type: 'choice', instructions, criteria }
}

/** Position on ordered levels; criteria is the ordered level descriptions. */
export function scoreQuestion(instructions, levels) {
  return { type: 'score', instructions, criteria: levels }
}
