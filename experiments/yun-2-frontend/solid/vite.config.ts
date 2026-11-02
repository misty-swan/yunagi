import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';

export default defineConfig({
  plugins: [solid()],
  clearScreen: false,
  server: { port: 5174, strictPort: true },
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
