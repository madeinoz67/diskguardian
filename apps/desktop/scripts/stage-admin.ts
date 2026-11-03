// Stage the measurement helper for bundling.
//
// Tauri's `externalBin` copies a prebuilt binary into Contents/MacOS/, but it does not
// build one: it wants a target-triple-suffixed file under src-tauri/binaries/. This
// script builds the workspace member and puts it where the bundler looks, so
// `bun run tauri:build` produces a bundle whose admin runner sits beside the app
// binary — which is also the first place scan_restricted looks for it at runtime.
//
// `--target <triple>` stages for a cross-built app: a release matrix building
// x86_64-apple-darwin on an arm64 runner needs the helper under that triple too, or the
// bundler refuses the externalBin. Without it the host triple is used.
import { $ } from 'bun'

// Parse `--target <triple>` / `--target=<triple>` without pulling in an arg library.
const argv = process.argv.slice(2)
let target = ''
for (let i = 0; i < argv.length; i += 1) {
  const arg = argv[i] ?? ''
  if (arg === '--target') target = argv[i + 1] ?? ''
  else if (arg.startsWith('--target=')) target = arg.slice('--target='.length)
}

const host = (await $`rustc -vV`.text()).match(/host: (\S+)/)?.[1]
if (!host) throw new Error('rustc -vV did not report a host triple; is a Rust toolchain installed?')

const triple = target || host

// From apps/desktop, where bun run executes this script.
if (triple === host) {
  await $`cargo build -p diskguardian-admin --release --locked`
  await $`mkdir -p src-tauri/binaries`
  await $`cp ../../target/release/diskguardian-admin src-tauri/binaries/diskguardian-admin-${triple}`
} else {
  // A cross build lands under target/<triple>/release, not target/release.
  await $`cargo build -p diskguardian-admin --release --locked --target ${triple}`
  await $`mkdir -p src-tauri/binaries`
  await $`cp ../../target/${triple}/release/diskguardian-admin src-tauri/binaries/diskguardian-admin-${triple}`
}

console.log(`staged diskguardian-admin for ${triple}`)
