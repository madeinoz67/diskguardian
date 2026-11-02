// Stage the measurement helper for bundling.
//
// Tauri's `externalBin` copies a prebuilt binary into Contents/MacOS/, but it does not
// build one: it wants a target-triple-suffixed file under src-tauri/binaries/. This
// script builds the workspace member and puts it where the bundler looks, so
// `bun run tauri:build` produces a bundle whose admin runner sits beside the app
// binary — which is also the first place scan_restricted looks for it at runtime.
import { $ } from 'bun'

const host = (await $`rustc -vV`.text()).match(/host: (\S+)/)?.[1]
if (!host) throw new Error('rustc -vV did not report a host triple; is a Rust toolchain installed?')

// From apps/desktop, where bun run executes this script.
await $`cargo build -p diskguardian-admin --release --locked`
await $`mkdir -p src-tauri/binaries`
await $`cp ../../target/release/diskguardian-admin src-tauri/binaries/diskguardian-admin-${host}`
console.log(`staged diskguardian-admin for ${host}`)
