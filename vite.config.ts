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
  // 画面のコードから読める環境変数（VITE_ と、Tauri が渡す TAURI_ENV_ で始まるもの）。
  // Vite は名前の先頭が一致するかで照合する（"*" のような書き方は使えない）
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // 出力する JavaScript・CSS が前提にする WebView の下限（Windows は Chromium 系、macOS・Linux は WebKit 系）。
    // Tauri の手引きの例（chrome105・safari13）は Tauri 2 が動く OS の下限に合わせた値で、
    // Svelte 5・Vite 8 が前提にする版（Vite 8 の既定 "baseline-widely-available" の chrome111・safari16.4）より古い。
    // そこで Vite 8 の既定と同じ下限にする。対応 OS（docs/Requirements-Document.md 8章）の WebView はどれもこれ以上：
    // - Windows 10/11：WebView2 は自動で更新される Chromium（111 以上）
    // - macOS 直近3世代：Safari 18 以上の WebKit
    // - Linux：Tauri 2 は webkit2gtk-4.1 を使い、Ubuntu LTS・Fedora は WebKitGTK 2.40（Safari 16.4 と同時期）以上を配る
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome111" : "safari16.4",
    // デバッグ版のビルドでは縮小せず、ソースマップを付ける。
    // 縮小は Vite 8 の既定の方法を使う（推奨の "esbuild" は Vite 8 では別に入れる必要があるため）
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
  // テストでは Svelte のブラウザ用の版を使う（mount を使うため）。Vite の既定の条件は残す
  resolve: process.env.VITEST ? { conditions: ["browser", ...defaultClientConditions] } : undefined,
  test: {
    environment: "jsdom",
    // install/ はインストールスクリプトの試験（install/install.test.ts。配布手順書 4.5）、
    // scripts/release/ は配布用 CI が使うスクリプトの試験（配布手順書 3章）、
    // site/ は公式サイトのビルドと出力の検査（配布手順書 2章「公式サイトの作り方」）
    include: ["src/**/*.test.ts", "install/**/*.test.ts", "scripts/**/*.test.ts", "site/**/*.test.ts"],
    // テストでは CSS を空にするが、色のトークンとフォーム部品は中身を確かめる（src/styles/tokens.test.ts・forms.test.ts）
    css: { include: [/src\/styles\/(tokens|base|forms)\.css/] },
  },
});
