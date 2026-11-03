<script lang="ts">
  // アプリの画面：コアとつなぎ（session）、画面の外枠（AppShell）に本文を差し込む。
  // 本文は、整理モードの3領域（src/board/）とノート（src/note/）。どちらも、もう一方のモードの間も
  // 消さずに隠す（入力中の行・カーソルと自動保存の待ちを保つため。画面設計書 4章）。モードを移ると、
  // ノートではエディタに、整理モードでは移る前にフォーカスのあった所に戻る。
  // 入力中の状態（項目・振り返り・ノート）は1つの InputTracker にまとめて知らせる
  // （docs/design/01-architecture.md 4.1「入力中の状態の知らせ」）。
  // データベースを開くまで（とバックアップから戻した結果を確かめるまで）は、外枠の代わりに起動時の画面
  // （src/startup/）を窓全体に出す。開く処理の状態は、起動時の画面の data-startup に出す（テストに使う）。定期バックアップの失敗（backup-failed）は上部のバーの知らせに出す。
  import type { EditorView } from "@codemirror/view";
  import type { NoteSource } from "./lib/bindings";
  import { onDestroy, untrack } from "svelte";
  import { coreEvents } from "./lib/core";
  import { t } from "./lib/i18n";
  import { SaveStatus } from "./lib/autosave.svelte";
  import { isMacPlatform } from "./lib/keys";
  import { applyPrefs, prefs } from "./lib/prefs.svelte";
  import { startSession } from "./lib/session.svelte";
  import { Shell } from "./lib/shell.svelte";
  import { InputTracker } from "./lib/ui_report.svelte";
  import Board from "./board/Board.svelte";
  import NoteView from "./note/NoteView.svelte";
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
  const tracker = new InputTracker(save);
  onDestroy(() => tracker.stop());
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

  // アプリが前面から外れたら、まだ保存していない入力をすぐ送り、入力中を終える
  function onWindowBlur() {
    void save.flushAll();
    tracker.setActive(false);
  }

  // ---- モードを移るときのフォーカス ----

  let noteView = $state<EditorView | null>(null);
  /** 整理モードで最後にフォーカスのあった所（ノートモードから戻るときに戻す）。マウスでモードを移る
   *  ときはボタンにフォーカスが移るので、移る時点ではなく、整理モードの中のフォーカスを追い続ける。 */
  let boardFocus: Element | null = null;
  let lastMode = shell.mode;
  $effect(() => {
    const mode = shell.mode;
    untrack(() => {
      if (mode === lastMode) return;
      lastMode = mode;
      if (mode === "note") noteView?.focus();
      else if (boardFocus instanceof HTMLElement && boardFocus.isConnected) boardFocus.focus();
    });
  });
  // 整理モードに戻るときは、隠す前にノートからフォーカスを外す（戻す先が無くても、隠れたエディタに
  // 打ち込まないため。隠した後では外せないことがある）
  $effect.pre(() => {
    if (shell.mode !== "board") return;
    untrack(() => {
      const a = document.activeElement;
      if (a instanceof HTMLElement && noteView?.dom.contains(a)) a.blur();
    });
  });

  // ---- 項目から送り元のノートの行へ ----

  /** 項目の「← NOTE」で移る先（ノートがその日を読み終えて移ったら `null` に戻す）。 */
  let noteJump = $state<NoteSource | null>(null);
  function openNoteSource(source: NoteSource) {
    // 押すたびに移る（同じ行へ2回目に押したときも）
    noteJump = { ...source };
    if (session.date !== source.date) session.show(source.date);
    shell.setMode("note");
  }
  /** その日のノートから送った項目（ノートが送信済みの印を求め直すため。送った項目はノートと同じ日に作る）。 */
  const boardItems = $derived(
    [...(session.board?.todos ?? []), ...(session.board?.blockers ?? [])]
      .filter((bi) => bi.item.origin === "note")
      .map((bi) => bi.item.id)
      .join(","),
  );

  // 検索パレット・設定・変更履歴・ふりかえりは、それぞれの画面の課題で開くようにする
  function onCommand(_c: ShellCommand) {}
</script>

<svelte:window onblur={onWindowBlur} onfocus={() => tracker.setActive(true)} />

{#if !ready}
  <StartupScreen startup={session.startup} actions={startupActions} />
{:else}
  <AppShell {session} {save} {shell} {mac} notice={notice.text} {onCommand}>
    <main
      class="content"
      data-mode={shell.mode}
      data-date={session.date ?? ""}
    >
      <div class="mode" hidden={shell.mode !== "board"} onfocusin={(e) => (boardFocus = e.target as Element)}>
        <Board {session} {save} {notice} narrow={shell.narrow} {mac} {tracker} onOpenNoteSource={openNoteSource} />
      </div>
      <div class="mode" hidden={shell.mode !== "note"}>
        <NoteView
          date={session.board?.date ?? null}
          {save}
          presence={tracker.note}
          rev={session.board?.noteRev ?? 0}
          onActivity={tracker.activity}
          visible={shell.mode === "note"}
          narrow={shell.narrow}
          {mac}
          {notice}
          items={boardItems}
          jump={noteJump}
          onJumped={() => (noteJump = null)}
          bind:view={noteView}
        />
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
