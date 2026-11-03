<script lang="ts">
  // 画面の中身は「2. 画面」プロジェクトの課題で作る。ここではコアとつなぐだけ
  // （開く処理の状態を data-startup に出す。本番ビルドでの呼び出しの確認とテストに使う）。
  import { onDestroy } from "svelte";
  import { startSession } from "./lib/session.svelte";

  const session = startSession();
  onDestroy(() => void session.stop());
</script>

<main class="app" data-startup={session.startup?.state ?? "unknown"}></main>

<style>
  :global(html, body) {
    margin: 0;
    height: 100%;
  }

  .app {
    height: 100vh;
  }
</style>
