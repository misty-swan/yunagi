import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    target: 'es2022',
    chunkSizeWarningLimit: 1000,
    // サイズ比較のため、両版共通のライブラリを別ファイルに分ける（cm = CodeMirror、tauri = Tauri API）
    rolldownOptions: {
      output: {
        codeSplitting: {
          groups: [
            { name: 'cm', test: /node_modules[\\/].*(@codemirror|@lezer|codemirror|style-mod|w3c-keyname|crelt|@marijn)/ },
            { name: 'tauri', test: /node_modules[\\/].*@tauri-apps/ },
          ],
        },
      },
    },
  },
});
