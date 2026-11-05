import { defineConfig } from 'vite'

// Tauri drives the dev server: fixed port, no fallback so a stale server can never
// silently serve the wrong build.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    target: 'safari15',
    // `esbuild` is not a Vite 8 option. Vite 8 bundles with rolldown and minifies with
    // oxc, and it dropped its own esbuild dependency — so the old value did not fall back
    // to a default, it tried to import a package that is no longer installed and failed
    // the build. `oxc` is the named equivalent: minify on, same intent, the bundler that
    // actually ships.
    minify: 'oxc',
    sourcemap: false,
  },
})
