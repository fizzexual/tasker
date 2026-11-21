import { defineConfig } from "vite";

// Tauri expects a fixed port and its own Rust sources left alone by the watcher.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "es2022",
    sourcemap: false,
  },
});
