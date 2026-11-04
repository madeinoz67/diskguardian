// The version number is hand-maintained in three places: the workspace manifest, the
// desktop package.json, and the Tauri bundle config. Nothing in the build compares
// them — the bundle embeds tauri.conf.json's version while cargo stamps the
// workspace's, so a half-done bump ships artifacts that disagree about what they are.
// This guard makes the disagreement fail `bun test` instead (governance LOW-3,
// 2026-11-03). Paths are resolved from this file, never from the cwd, so the test
// behaves the same from `apps/desktop` and from the repo root.
import { test, expect } from 'bun:test'
import { readFileSync } from 'node:fs'

const repoRoot = new URL('../../../', import.meta.url)
const read = (relative: string): string => readFileSync(new URL(relative, repoRoot), 'utf8')

test('the three version surfaces agree', () => {
  // The first `version = "…"` line in the root manifest is [workspace.package]'s —
  // the [workspace] table above it carries no version key of its own.
  const workspace = read('Cargo.toml').match(/^version = "(.+)"$/m)?.[1]
  const pkg = (JSON.parse(read('apps/desktop/package.json')) as { version?: string }).version
  const tauri = (JSON.parse(read('apps/desktop/src-tauri/tauri.conf.json')) as {
    version?: string
  }).version

  expect(workspace, 'workspace [package].version must exist').toBeTruthy()
  expect(pkg, 'apps/desktop/package.json version').toBe(workspace)
  expect(tauri, 'tauri.conf.json version').toBe(workspace)
})
