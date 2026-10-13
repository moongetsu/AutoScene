/// <reference types="vitest/config" />
import path from "node:path"
import { defineConfig } from "vite"
import react from "@vitejs/plugin-react"
import tailwindcss from "@tailwindcss/vite"

const repoRoot = path.resolve(import.meta.dirname, "..")
const fixtures = path.join(repoRoot, "fixtures", "out")

// Tauri expects a fixed port and must not have the terminal cleared.
export default defineConfig(({ command }) => ({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    // The browser mock plays the barcoded test fixtures straight from fixtures/out.
    fs: { allow: [import.meta.dirname, fixtures] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  define:
    command === "serve"
      ? { "import.meta.env.VITE_FIXTURES_URL": JSON.stringify(`/@fs/${fixtures.split(path.sep).join("/")}/`) }
      : {},
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "./src") } },
  build: {
    target: "es2023",
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: true,
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
  },
}))
