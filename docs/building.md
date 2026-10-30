# Building

## Prerequisites

- **Rust** (stable) with `cargo`
- **Bun** (frontend tooling)
- **Xcode command line tools** (`xcode-select --install`)

## Build the app

```bash
bun install --cwd apps/desktop
bun x --cwd apps/desktop tauri build
```

The bundle lands at:

```
apps/desktop/src-tauri/target/release/bundle/macos/DiskGuardian.app
```

For development with hot reload:

```bash
cd apps/desktop && bun run tauri:dev
```

## Without a native window

The frontend runs in a plain browser against a synthetic tree, which is the fastest way to
work on the sunburst:

```bash
cd apps/desktop && bun run dev
```

## Tests

```bash
cargo test -p diskguardian-core       # engine: sizes, links, permissions, guards
cd apps/desktop && bun test           # sunburst layout maths
cd apps/desktop && bunx tsc --noEmit  # frontend types
```

The engine tests build real files on disk, because the thing under test is the
filesystem's own accounting. A mocked tree would prove nothing.
