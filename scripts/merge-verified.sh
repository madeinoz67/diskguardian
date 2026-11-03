#!/usr/bin/env bash
# merge-verified.sh — the merge gate as a tool.
#
# Ported from BenchWeave 2026-11-03, the day after the first release. diskguardian
# merged with `git merge --no-ff` and read no rollup at all, which is how a Windows
# compile break and an ext4-only test reached main while the local battery was
# reported green. Gates prove things about the machine they run on; the rollup is
# the only evidence that covers the platforms we ship claims about.
#
# Never merge on a piped or tailed view: `gh pr checks --watch | tail` hides
# failing lanes and masks the exit code (2026-09-23: two merges landed with
# three red lanes behind a tailed view). This script watches the checks to
# terminal state, then reads the COMPLETE rollup, refuses on any `fail` or
# `pending` line — and on a PR that reports no checks at all — and only then
# merges. A background watcher's exit code is advisory; the rollup is the
# verdict.
#
# Usage:
#   scripts/merge-verified.sh                       # PR inferred from the current branch
#   scripts/merge-verified.sh <pr-number>           # repository inferred from the remote
#   scripts/merge-verified.sh <owner/repo> <pr>     # both explicit
#
# The merge is a merge commit (--merge, the house style); the remote branch is
# NOT deleted — remote deletions take the owner's word.
#
# Exits 0 after merging; exits non-zero WITHOUT merging when any check is
# failing or pending, the rollup is empty/unreadable, or the PR is not open.
set -uo pipefail

say() { printf 'merge-verified: %s\n' "$*" >&2; }
die() { printf 'merge-verified: %s\n' "$*" >&2; exit 1; }

# --- resolve repository and PR ------------------------------------------------
repo=""
pr=""
for arg in "$@"; do
  case "$arg" in
    ''|*[!0-9]*) repo="$arg" ;;
    *) pr="$arg" ;;
  esac
done
[ -z "$repo" ] || case "$repo" in
  */*) ;;
  *) die "unrecognized argument '$repo' (want owner/repo and/or a PR number)" ;;
esac

if [ -z "$repo" ]; then
  repo="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null)" ||
    die "cannot infer the repository (gh repo view failed) — pass owner/repo explicitly"
fi
if [ -z "$pr" ]; then
  pr="$(gh pr view -R "$repo" --json number -q .number 2>/dev/null)" ||
    die "cannot infer a PR from the current branch — pass the PR number"
fi

# --- the PR must be open ------------------------------------------------------
state="$(gh pr view "$pr" -R "$repo" --json state -q .state 2>/dev/null)" ||
  die "PR $repo#$pr is not readable"
[ "$state" = "OPEN" ] || die "PR $repo#$pr is $state, not OPEN — nothing to merge"

# --- watch to terminal state (exit code advisory, output suppressed) ----------
say "$repo#$pr: watching checks to terminal state..."
if gh pr checks "$pr" -R "$repo" --watch >/dev/null 2>&1; then
  say "watch exited 0 (advisory only — the full rollup below is the verdict)"
else
  say "watch exited non-zero (advisory only — reading the full rollup before deciding)"
fi

# --- the verdict: the COMPLETE rollup, read in full ---------------------------
# gh pr checks emits one tab-separated line per check: name, status, elapsed,
# URL. Status values: pass, fail, pending, skipping (skipping is a healthy
# lane outcome and does not block).
rollup="$(gh pr checks "$pr" -R "$repo" 2>&1)" || true
printf '\n--- full rollup: gh pr checks %s -R %s ---\n%s\n--- end rollup ---\n\n' \
  "$pr" "$repo" "$rollup"

total="$(printf '%s\n' "$rollup" | awk -F'\t' 'NF >= 2 && $2 ~ /^(pass|fail|pending|skipping)$/ {n++} END {print n + 0}')"
fails="$(printf '%s\n' "$rollup" | awk -F'\t' '$2 == "fail" {n++} END {print n + 0}')"
pend="$(printf '%s\n' "$rollup" | awk -F'\t' '$2 == "pending" {n++} END {print n + 0}')"

[ "$total" -gt 0 ] || die "no checks reported for $repo#$pr — refusing to merge on an empty rollup"
[ "$fails" -eq 0 ] || die "$fails failing check(s) in the rollup — NOT merging"
[ "$pend" -eq 0 ] || die "$pend pending check(s) in the rollup — NOT merging"
say "rollup read in full: $total checks, 0 fail, 0 pending — merging"

# --- merge --------------------------------------------------------------------
gh pr merge "$pr" -R "$repo" --merge ||
  die "gh pr merge exited non-zero — verify the PR state before retrying"
say "merged $repo#$pr"
