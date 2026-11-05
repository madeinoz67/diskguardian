#!/usr/bin/env bash
# merge-verified.sh — the merge gate as a tool.
#
# Ported from BenchWeave 2026-11-03, the day after the first release. diskguardian
# merged with `git merge --no-ff` and read no rollup at all, which is how a Windows
# compile break and an ext4-only assertion reached main while the local battery was
# reported green. Gates prove things about the machine they run on; the rollup is the
# only evidence that covers the platforms the claims are about.
#
# Never merge on a piped or tailed view: `gh pr checks --watch | tail` hides failing
# lanes and masks the exit code. This script polls to terminal state, prints the
# COMPLETE rollup, refuses on any `fail` or `pending` line — and on a PR that reports
# no checks at all — and only then merges. A watcher's exit code is advisory; the
# rollup is the verdict.
#
# Usage:
#   scripts/merge-verified.sh                  # PR inferred from the current branch
#   scripts/merge-verified.sh <pr-number>      # repository inferred from the remote
#   scripts/merge-verified.sh <owner/repo> <pr>
#
# The merge is a SQUASH (--squash, principal's ruling 2026-11-05). The `main` ruleset
# permits only squash and rebase, so a merge commit is refused outright: "Merge commits
# are not allowed on this repository". One commit per pull request also reads cleanly
# next to the generated changelog, which is organised by pull request. The remote branch
# is NOT deleted — remote deletions take the principal's word.
#
# Exits 0 only after merging.
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

# --- poll to terminal state, bounded -----------------------------------------
#
# Two windows this has to cover, both found by running it rather than reading it:
# a push starts a stretch where NO checks exist yet (`gh pr checks --watch` exits
# immediately with "no checks reported on the <branch> branch"), and once they appear
# most are still pending. Breaking on "any check exists" read a rollup of three
# pending lanes and refused. So: poll until there is at least one check AND none is
# pending, or the deadline passes — in which case the verdict below refuses on the
# pending lanes rather than pretending they were green.
deadline=$((SECONDS + 900))
while :; do
  rollup="$(gh pr checks "$pr" -R "$repo" 2>&1)" || true
  total="$(printf '%s\n' "$rollup" | awk -F'\t' 'NF >= 2 && $2 ~ /^(pass|fail|pending|skipping)$/ {n++} END {print n + 0}')"
  pend="$(printf '%s\n' "$rollup" | awk -F'\t' '$2 == "pending" {n++} END {print n + 0}')"
  if [ "$total" -gt 0 ] && [ "$pend" -eq 0 ]; then break; fi
  [ "$SECONDS" -ge "$deadline" ] && break
  if [ "$total" -eq 0 ]; then
    say "no checks yet ($((deadline - SECONDS))s of budget left) — CI may still be starting"
  else
    say "$pend of $total checks still pending — waiting"
  fi
  sleep 20
done

# --- the verdict: the COMPLETE rollup, read in full ---------------------------
# gh pr checks emits one tab-separated line per check: name, status, elapsed, URL.
# Status values: pass, fail, pending, skipping (skipping is a healthy lane outcome).
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
gh pr merge "$pr" -R "$repo" --squash ||
  die "gh pr merge exited non-zero — verify the PR state before retrying"
say "merged $repo#$pr"
