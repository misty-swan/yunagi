<script lang="ts">
  // アプリの画面：コアとつなぎ（session）、画面の外枠（AppShell）に本文を差し込む。
  // 本文（整理モードの3領域・ノート）は、それぞれの画面の課題で作る。ここでは差し込む場所まで。
  // 開く処理の状態を本文の data-startup に出す（本番ビルドでの呼び出しの確認とテストに使う）。
  import { onDestroy } from "svelte";
  import { SaveStatus } from "./lib/autosave.svelte";
  import { isMacPlatform } from "./lib/keys";
  import { applyPrefs, prefs } from "./lib/prefs.svelte";
  import { startSession } from "./lib/session.svelte";
  import { Shell } from "./lib/shell.svelte";
  import AppShell from "./shell/AppShell.svelte";
  import type { ShellCommand } from "./shell/commands";

  const session = startSession();
  onDestroy(() => void session.stop());
  const save = new SaveStatus();
  const shell = new Shell();
  const mac = isMacPlatform();

  $effect(() => applyPrefs(prefs.theme, prefs.lang));

  // 検索パレット・設定・変更履歴・ふりかえりは、それぞれの画面の課題で開くようにする
  function onCommand(_c: ShellCommand) {}
</script>

<AppShell {session} {save} {shell} {mac} {onCommand}>
  <main
    class="content"
    data-startup={session.startup?.state ?? "unknown"}
    data-mode={shell.mode}
    data-date={session.date ?? ""}
  ></main>
</AppShell>

<style>
  .content {
    height: 100%;
  }
</style>
