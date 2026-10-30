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
    minify: 'esbuild',
    sourcemap: false,
  },
})
