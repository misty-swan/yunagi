/// <reference types="vitest/config" />
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defaultClientConditions, defineConfig } from "vite";

// Tauri の Vite の推奨設定に従う：https://v2.tauri.app/start/frontend/vite/
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
  },
  // 画面のコードから読める環境変数（VITE_ と、Tauri が渡す TAURI_ENV_ で始まるもの）
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    // Tauri の WebView に合わせる（Windows は Chromium 系、macOS・Linux は WebKit 系）
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    // デバッグ版のビルドでは縮小せず、ソースマップを付ける。
    // 縮小は Vite 8 の既定の方法を使う（推奨の "esbuild" は Vite 8 では別に入れる必要があるため）
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
  // テストでは Svelte のブラウザ用の版を使う（mount を使うため）。Vite の既定の条件は残す
  resolve: process.env.VITEST ? { conditions: ["browser", ...defaultClientConditions] } : undefined,
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
  },
});
