<script lang="ts">
  // アプリの画面：コアとつなぎ（session）、画面の外枠（AppShell）に本文を差し込む。
  // 本文は、整理モードの3領域（src/board/）とノート（src/note/）。どちらも、もう一方のモードの間も
  // 消さずに隠す（入力中の行・カーソルと自動保存の待ちを保つため。画面設計書 4章）。モードを移ると、
  // ノートではエディタに、整理モードでは移る前にフォーカスのあった所に戻る。
  // 入力中の状態（項目・振り返り・ノート）は1つの InputTracker にまとめて知らせる
  // （docs/design/01-architecture.md 4.1「入力中の状態の知らせ」）。
  // データベースを開くまで（とバックアップから戻した結果を確かめるまで）は、外枠の代わりに起動時の画面
  // （src/startup/）を窓全体に出す。開く処理の状態は、起動時の画面の data-startup に出す（テストに使う）。定期バックアップの失敗（backup-failed）は上部のバーの知らせに出す。
  // 持ち越し（S-03。src/carryover/）は、開いたときと前面に出したときに確かめ、外枠の上に重ねて出す。
  // 検索パレット（S-04。src/palette/）も外枠の上に重ね、選んだもの（日付・コマンド・検索結果）はここで行う。
  // ふりかえり（S-08。src/archive/）は、開いている間、整理モードとノートの代わりに本文へ出す（二つは消さずに隠す）。
  import type { EditorView } from "@codemirror/view";
  import type { LogicalDate, NoteSource } from "./lib/bindings";
  import {
    Palette,
    type BoardTarget,
    type NoteFind,
    type PaletteChoice,
    type PaletteCommandId,
  } from "./lib/palette.svelte";
  import PalettePanel from "./palette/Palette.svelte";
  import { flushSync, onDestroy, untrack } from "svelte";
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
  import type { ArchiveControl, ShellCommand } from "./shell/commands";
  import { ArchiveScreen } from "./lib/archive.svelte";
  import ArchiveView from "./archive/Archive.svelte";
  import { Carryover } from "./lib/carryover.svelte";
  import CarryoverPanel from "./carryover/CarryoverPanel.svelte";
  import { reloadPage } from "./lib/page";
  import { SettingsStore } from "./lib/settings.svelte";
  import { SettingsPanel } from "./lib/settings_panel.svelte";
  import { DataSettings } from "./lib/settings_data.svelte";
  import SettingsDialog from "./settings/SettingsDialog.svelte";

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

  // ---- 設定（S-05。画面設計書 7章） ----

  const settingsStore = new SettingsStore();
  const settingsPanel = new SettingsPanel();
  // 保存先の変更・選び直し・復元の前に、まだ保存していない入力を送る。選び直しと復元の後は読み込み直す
  const dataSettings = new DataSettings({
    flush: () => save.flushAll(),
    unsaved: () => save.failing || save.saving,
    reload: reloadPage,
    today: () => session.today,
  });
  onDestroy(() => dataSettings.stop());
  // データベースを開けたら、外枠を出す前に設定を読む（保存した値で描く。等幅のノートの書体は、読み込み終えて
  // から描く）。読めなければ既定の値で続ける
  let settingsStarted = false;
  $effect(() => {
    if (!ready) {
      // 開き直すことになった（別のデータベースを開くかもしれない）ので、開けたらもう一度読む
      if (settingsStarted) untrack(() => settingsStore.unload());
      settingsStarted = false;
      return;
    }
    if (settingsStarted) return;
    settingsStarted = true;
    untrack(() => {
      void settingsStore.load(() => {
        // 保存してあった、広いウィンドウでのサイドバーの開閉（最初に外枠を描くときから当てる）。
        // 利用者が開け閉めしたら保存する。保存できなかったら、画面の開閉も戻す
        shell.setWideOpen(prefs.sidebarOpen);
        shell.onWideOpenChange = (open) =>
          void settingsStore.save({ sidebarOpen: open }, { silent: true }).then((ok) => {
            if (!ok) shell.setWideOpen(prefs.sidebarOpen);
          });
      });
    });
  });
  const shown = $derived(ready && settingsStore.loaded);

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

  $effect(() => applyPrefs(prefs.theme, prefs.lang, prefs.fontSize));

  // ---- ふりかえり（S-08。画面設計書 9章） ----

  // 開いている間は、整理モードとノートを隠す（消さない）。選んだ日はふりかえりの中だけで持ち、
  // Enter でその日を開くまで、表示している日は動かさない
  const archive = new ArchiveScreen({ today: () => session.today, flush: () => save.flushAll() });
  archive.start();
  onDestroy(() => archive.stop());

  /** 開く。最初に選ぶ日は、いま見ている日（今日なら、コアが決める前の日）。 */
  function openArchive() {
    if (archive.open) {
      archive.focusSeq++;
      return;
    }
    const showing = session.showing;
    archive.show(showing !== null && showing !== session.today ? showing : null);
  }

  /** 閉じて、開く前にフォーカスのあった所へ戻す（`focus` が偽なら戻さない。ほかの画面へ移るとき）。 */
  function closeArchive(focus = true) {
    if (!archive.open) return;
    const back = archive.returnFocus;
    archive.returnFocus = null;
    archive.close();
    // 隠していた本文を出し終えてから戻す
    flushSync();
    if (!focus) return;
    if (back instanceof HTMLElement && back.isConnected && back.closest("[hidden]") === null) back.focus();
    else if (shell.mode === "note") noteView?.focus();
    else if (boardFocus instanceof HTMLElement && boardFocus.isConnected) boardFocus.focus();
  }

  /** 選んだ日のボードを開く（Enter・ダブルクリック・「Open ↵」）。 */
  function openArchiveDay(date: LogicalDate) {
    closeArchive(false);
    shell.setMode("board");
    showDate(date);
    flushSync();
    if (boardFocus instanceof HTMLElement && boardFocus.isConnected) boardFocus.focus();
  }

  const archiveControl = $derived<ArchiveControl | null>(
    archive.open
      ? {
          date: archive.sel,
          onSelect: (d) => archive.select(d),
          onWeek: (w) => archive.moveWeek(w),
          onClose: () => closeArchive(),
          onOpenDay: openArchiveDay,
        }
      : null,
  );

  // ---- 持ち越し（S-03。画面設計書 5章） ----

  // 出すときは、今日の整理モードの上に重ねる。一覧は、まだ保存していない入力を送り終えてから読む
  // （前の日の項目の本文・完了を送る前の内容で持ち越さないため）。ふりかえりを開いていれば閉じる
  const carry = new Carryover(async () => {
    const flushed = save.flushAll();
    // ふりかえりを開いていたら、その前にフォーカスのあった所へ、閉じた後に戻す
    if (archive.open) {
      carry.returnFocus = archive.returnFocus;
      archive.returnFocus = null;
      closeArchive(false);
    }
    // ノートモードから移るときは、閉じた後に、隠れたノートではなく整理モードで前にフォーカスのあった所へ戻す
    if (shell.mode !== "board") carry.returnFocus = boardFocus;
    shell.setMode("board");
    const today = session.today;
    if (today !== null && session.showing !== today) session.show(today);
    await flushed;
  });
  onDestroy(() => void carry.stop());
  // データベースを開いたら（起動は前面に出したことに数える）確かめる
  let carryStarted = false;
  $effect(() => {
    if (!ready || carryStarted) return;
    carryStarted = true;
    untrack(() => void carry.check());
  });

  // アプリが前面から外れたら、まだ保存していない入力をすぐ送り、入力中を終える
  function onWindowBlur() {
    void save.flushAll();
    tracker.setActive(false);
  }

  // 前面に出したら、入力中に戻し、持ち越しを出すかを確かめる（日付が変わった後・後回しの1回だけの再表示）
  function onWindowFocus() {
    tracker.setActive(true);
    if (ready) void carry.check();
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

  /** 項目の「← NOTE」・パレットのノートの結果で移る先（ノートがその日を読み終えて移ったら `null` に戻す）。 */
  let noteJump = $state<NoteSource | NoteFind | null>(null);
  function openNoteSource(source: NoteSource) {
    // 押すたびに移る（同じ行へ2回目に押したときも）
    noteJump = { ...source };
    if (session.date !== source.date) session.show(source.date);
    shell.setMode("note");
  }
  // 移り終える前に、ほかの日へ移った・整理モードに戻ったら、移るのをやめる（後で思わぬときに
  // カーソルを動かさないため）
  $effect(() => {
    const j = noteJump;
    const showing = session.showing;
    const mode = shell.mode;
    if (j !== null && (showing !== j.date || mode !== "note")) untrack(() => (noteJump = null));
  });
  /** その日のノートから送った項目（ノートが送信済みの印を求め直すため。送った項目はノートと同じ日に作る）。 */
  const boardItems = $derived(
    [...(session.board?.todos ?? []), ...(session.board?.blockers ?? [])]
      .filter((bi) => bi.item.origin === "note")
      .map((bi) => bi.item.id)
      .join(","),
  );

  // ---- 検索パレット（S-04。画面設計書 6章） ----

  const palette = new Palette();
  // 閉じると、待っている検索もやめる
  onDestroy(() => palette.close());
  /** 検索結果から移る、整理モードの中の先（Board が移り終えたら `null` に戻す）。 */
  let boardTarget = $state<BoardTarget | null>(null);
  let targetSeq = 0;
  /** 増えたら、上部のバーの保存できていない原因の表示を開く。 */
  let saveDetailRequest = $state(0);
  // 持ち越しを出したら、パレットと設定は閉じる（持ち越しを閉じた後は、開く前の所へ戻る）
  $effect(() => {
    if (!carry.open) return;
    untrack(() => {
      if (palette.open) {
        carry.returnFocus = palette.returnFocus;
        palette.returnFocus = null;
        palette.close();
      } else if (settingsPanel.open) {
        carry.returnFocus = settingsPanel.returnFocus;
        settingsPanel.returnFocus = null;
        settingsPanel.close();
      }
    });
  });
  // 移り終える前に、ほかの日へ移ったら、移るのをやめる
  $effect(() => {
    const tg = boardTarget;
    const showing = session.showing;
    if (tg !== null && showing !== tg.date) untrack(() => (boardTarget = null));
  });

  function showDate(date: LogicalDate) {
    if (session.showing !== date) session.show(date);
  }

  function runPalette(choice: PaletteChoice) {
    if (choice.kind === "date") {
      // ふりかえりの間も、その日のボードを開く（日の移動は元のモードのまま。ふりかえりは閉じる）
      closeArchive();
      // モードは変えない（‹ › と同じ。画面設計書 3章「上部のバー」）
      showDate(choice.date);
      return;
    }
    if (choice.kind === "command") {
      runCommand(choice.id);
      return;
    }
    closeArchive(false);
    const { hit } = choice.view;
    if (hit.kind === "note") {
      noteJump = { date: hit.date, find: choice.query };
      showDate(hit.date);
      shell.setMode("note");
      return;
    }
    boardTarget =
      hit.kind === "reflection"
        ? { date: hit.date, find: choice.query, n: ++targetSeq }
        : { date: hit.date, itemId: hit.refId, n: ++targetSeq };
    showDate(hit.date);
    shell.setMode("board");
  }

  function runCommand(id: PaletteCommandId) {
    switch (id) {
      case "carryover":
        void carry.openOnRequest();
        return;
      case "saveDetail":
        saveDetailRequest++;
        return;
      case "today":
        // ふりかえりの間は、今日を選ぶ（Mod+T と同じ）
        if (session.today === null) return;
        if (archive.open) archive.select(session.today);
        else showDate(session.today);
        return;
      case "mode":
        // ふりかえりを開いていれば閉じてから切り替える（切り替えた先のモードが前に出る）
        closeArchive(false);
        shell.toggleMode();
        return;
      case "sidebar":
        shell.toggleSidebar();
        return;
      case "layoutA":
      case "layoutB":
      case "layoutC":
        void settingsStore.save({ layout: id === "layoutA" ? "A" : id === "layoutB" ? "B" : "C" });
        return;
      case "archive":
      case "agents":
      case "settings":
        openScreen(id);
        return;
    }
  }

  // 設定（S-05）とふりかえり（S-08）を開く。変更履歴（S-06）は、その画面の課題で開くようにする
  // （それまでは何もしない）。サイドバー・上部のバー・キー・パレットのどこから開くときも、ここを通る
  // （画面設計書 6章）。持ち越しを出している間は開かない（背後の操作を受け付けないため）
  function openScreen(c: Exclude<ShellCommand, "palette">) {
    if (carry.open) return;
    if (c === "settings") settingsPanel.show();
    else if (c === "dataSettings") settingsPanel.show("data");
    else if (c === "archive") openArchive();
  }

  function onCommand(c: ShellCommand) {
    if (c === "palette") {
      // 持ち越しを出している間は開かない（背後の操作を受け付けないため）
      if (!carry.open) palette.show();
      return;
    }
    openScreen(c);
  }
</script>

<svelte:window onblur={onWindowBlur} onfocus={onWindowFocus} />

{#if !ready}
  <StartupScreen startup={session.startup} actions={startupActions} />
{:else if shown}
  <!-- 持ち越し・パレット・設定を出している間は、背後の外枠と本文を操作できなくする -->
  <div inert={carry.open || palette.open || settingsPanel.open}>
    <AppShell
      {session}
      {save}
      {shell}
      {mac}
      notice={notice.text}
      {onCommand}
      {saveDetailRequest}
      archive={archiveControl}
    >
      <main
        class="content"
        data-mode={shell.mode}
        data-date={session.date ?? ""}
      >
        {#if archive.open}
          <div class="mode">
            <ArchiveView {archive} narrow={shell.narrow} onOpenDay={openArchiveDay} onClose={() => closeArchive()} />
          </div>
        {/if}
        <div class="mode" hidden={archive.open || shell.mode !== "board"} onfocusin={(e) => (boardFocus = e.target as Element)}>
          <Board
            {session}
            {save}
            {notice}
            narrow={shell.narrow}
            {mac}
            {tracker}
            onOpenNoteSource={openNoteSource}
            target={boardTarget}
            onTargetDone={() => (boardTarget = null)}
          />
        </div>
        <div class="mode" hidden={archive.open || shell.mode !== "note"}>
          <NoteView
            date={session.board?.date ?? null}
            {save}
            presence={tracker.note}
            rev={session.board?.noteRev ?? 0}
            onActivity={tracker.activity}
            visible={shell.mode === "note" && !archive.open}
            narrow={shell.narrow}
            {mac}
            {notice}
            items={boardItems}
            itemsDate={session.board?.date ?? null}
            jump={noteJump}
            onJumped={() => (noteJump = null)}
            bind:view={noteView}
          />
        </div>
      </main>
    </AppShell>
  </div>
  <PalettePanel
    {palette}
    today={session.today}
    context={{
      mode: shell.mode,
      sidebarOpen: shell.sidebarOpen,
      layout: prefs.layout,
      carryPending: carry.pending !== null,
      saveFailing: save.failing,
      mac,
    }}
    onRun={runPalette}
  />
  <SettingsDialog
    panel={settingsPanel}
    store={settingsStore}
    data={dataSettings}
    {mac}
    onOpenAgents={() => openScreen("agents")}
  />
  <CarryoverPanel {carry} />
{/if}

<style>
  .content,
  .mode {
    height: 100%;
  }
</style>
