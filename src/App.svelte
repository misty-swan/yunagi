<script lang="ts">
  // アプリの画面：コアとつなぎ（session）、画面の外枠（AppShell）に本文を差し込む。
  // 本文は、整理モードの3領域（src/board/）。ノートモードの本文は、ノートの画面の課題で作る。
  // 整理モードの本文は、ノートモードの間も消さずに隠す（入力中の行と自動保存の待ちを保つため）。
  // データベースを開くまで（とバックアップから戻した結果を確かめるまで）は、外枠の代わりに起動時の画面
  // （src/startup/）を窓全体に出す。開く処理の状態は、起動時の画面の data-startup に出す（テストに使う）。定期バックアップの失敗（backup-failed）は上部のバーの知らせに出す。
  import { onDestroy } from "svelte";
  import { coreEvents } from "./lib/core";
  import { t } from "./lib/i18n";
  import { SaveStatus } from "./lib/autosave.svelte";
  import { isMacPlatform } from "./lib/keys";
  import { applyPrefs, prefs } from "./lib/prefs.svelte";
  import { startSession } from "./lib/session.svelte";
  import { Shell } from "./lib/shell.svelte";
  import Board from "./board/Board.svelte";
  import { LONG_NOTICE_MS, Notice } from "./lib/notice.svelte";
  import { StartupActions } from "./lib/startup.svelte";
  import StartupScreen from "./startup/StartupScreen.svelte";
  import AppShell from "./shell/AppShell.svelte";
  import type { ShellCommand } from "./shell/commands";

  const session = startSession();
  onDestroy(() => void session.stop());
  const save = new SaveStatus();
  const shell = new Shell();
  const notice = new Notice();
  onDestroy(() => notice.clear());
  const mac = isMacPlatform();
  const startupActions = new StartupActions();
  // バックアップから戻した後（restored）は、利用者が確かめるまで起動時の画面のまま
  const ready = $derived(session.startup?.state === "ready");

  // 定期バックアップを作れなかった（1日に1回だけ届く。コアは自動で作り直す。画面設計書 3章「上部のバー」）
  let stopped = false;
  const unlistenBackup = coreEvents
    .onBackupFailed(() => {
      if (!stopped) notice.show(t().backupFailed, LONG_NOTICE_MS);
    })
    .catch(() => undefined);
  onDestroy(() => {
    stopped = true;
    void unlistenBackup.then((u) => u?.());
  });

  $effect(() => applyPrefs(prefs.theme, prefs.lang));

  // 検索パレット・設定・変更履歴・ふりかえりは、それぞれの画面の課題で開くようにする
  function onCommand(_c: ShellCommand) {}
</script>

{#if !ready}
  <StartupScreen startup={session.startup} actions={startupActions} />
{:else}
  <AppShell {session} {save} {shell} {mac} notice={notice.text} {onCommand}>
    <main
      class="content"
      data-mode={shell.mode}
      data-date={session.date ?? ""}
    >
      <div class="mode" hidden={shell.mode !== "board"}>
        <Board {session} {save} {notice} narrow={shell.narrow} {mac} />
      </div>
    </main>
  </AppShell>
{/if}

<style>
  .content,
  .mode {
    height: 100%;
  }
</style>
