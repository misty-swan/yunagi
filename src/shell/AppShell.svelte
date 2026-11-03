<script lang="ts">
  // 画面の外枠（画面設計書 3章「画面の骨組み」）：サイドバー・上部のバー・凪メーター・本文・キーの案内。
  // 本文（整理モードの3領域・ノート）は `children` に差し込む。
  //
  // 受け持つキー（操作仕様書 2章「共通」）：Mod+J、Mod+[ / Mod+]、Mod+T、Mod+K、Mod+,、Mod+\、
  // Mod+Shift+L。ほかの画面を開く操作は `onCommand` に渡す。
  import type { Snippet } from "svelte";
  import type { SaveStatus } from "../lib/autosave.svelte";
  import type { LogicalDate } from "../lib/bindings";
  import { addDays } from "../lib/dates";
  import { shellActionOf, type ShellAction } from "../lib/keys";
  import { meterScene, minutesOfDay } from "../lib/meter";
  import type { Session } from "../lib/session.svelte";
  import type { Mode, Shell } from "../lib/shell.svelte";
  import type { ShellCommand } from "./commands";
  import KeyHints from "./KeyHints.svelte";
  import NagiMeter from "./NagiMeter.svelte";
  import Sidebar from "./Sidebar.svelte";
  import TopBar from "./TopBar.svelte";

  let {
    session,
    save,
    shell,
    mac,
    notice = null,
    onCommand,
    saveDetailRequest = 0,
    children,
  }: {
    session: Session;
    save: SaveStatus;
    shell: Shell;
    mac: boolean;
    /** 上部のバーに短時間だけ出す知らせ。 */
    notice?: string | null;
    onCommand: (c: ShellCommand) => void;
    /** 増えたら、上部のバーの保存できていない原因の表示を開く。 */
    saveDetailRequest?: number;
    children?: Snippet;
  } = $props();

  /** 凪メーターの太陽の時刻のため、いまの時刻を 30 秒ごとに読み直す。 */
  const CLOCK_MS = 30_000;
  let now = $state(new Date());
  $effect(() => {
    const timer = setInterval(() => (now = new Date()), CLOCK_MS);
    return () => clearInterval(timer);
  });

  let innerWidth = $state(window.innerWidth);
  $effect(() => shell.setWidth(innerWidth));

  const date = $derived(session.date);
  // session.today は日付が変わるとすぐ進む（overview.today は読み直すまで前の日のまま）
  const today = $derived(session.today);
  const overview = $derived(session.overview);
  const scene = $derived(
    date !== null && today !== null && overview !== null
      ? meterScene(date, today, minutesOfDay(today, now), overview.counts)
      : null,
  );

  const from = () => session.showing ?? date;

  function show(next: LogicalDate | null) {
    if (next !== null) session.show(next);
  }

  function goToday() {
    show(today);
  }

  function setMode(mode: Mode) {
    shell.setMode(mode);
  }

  const actions: Record<ShellAction, () => void> = {
    toggleMode: () => shell.toggleMode(),
    // 読んでいる途中に続けて押しても1日ずつ進むよう、表示しようとしている日から数える
    prevDay: () => show(from() === null ? null : addDays(from()!, -1)),
    nextDay: () => show(from() === null ? null : addDays(from()!, 1)),
    today: goToday,
    palette: () => onCommand("palette"),
    settings: () => onCommand("settings"),
    sidebar: () => shell.toggleSidebar(),
    archive: () => onCommand("archive"),
  };

  function onKeydown(e: KeyboardEvent) {
    if (e.defaultPrevented) return;
    const action = shellActionOf(e, mac);
    if (action === null) return;
    e.preventDefault();
    // 押し続けたときの繰り返しでは動かさない（モードの切り替えが行き来し、日の移動の読み込みが重なるため）
    if (e.repeat) return;
    actions[action]();
  }
</script>

<svelte:window bind:innerWidth onkeydown={onKeydown} />

<div class="shell" data-mode={shell.mode} data-sidebar={shell.sidebarOpen ? "open" : "closed"}>
  {#if shell.sidebarOpen}
    <Sidebar
      {overview}
      {date}
      {today}
      saveFailed={save.failing}
      {mac}
      onShow={show}
      onToday={() => {
        setMode("board");
        goToday();
      }}
      {onCommand}
    />
  {/if}

  <div class="main" style:--sun-t={scene ? (4 + scene.progress * 92) / 100 : 0.8}>
    <div class="sky" aria-hidden="true"></div>
    <TopBar
      {date}
      {today}
      mode={shell.mode}
      showLogo={!shell.sidebarOpen}
      unseen={overview?.unseenChanges ?? 0}
      {notice}
      {save}
      {mac}
      onPrev={actions.prevDay}
      onNext={actions.nextDay}
      onToday={goToday}
      onMode={setMode}
      {onCommand}
      {saveDetailRequest}
    />
    {#if scene && overview}
      <NagiMeter {scene} counts={overview.counts} compact={shell.mode === "note"} />
    {/if}
    <div class="body">
      {@render children?.()}
    </div>
    <KeyHints mode={shell.mode} {mac} />
  </div>
</div>

<style>
  .shell {
    display: flex;
    height: 100vh;
  }

  .main {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    /* 空の光の高さ：水平線の少し上（整理モードの凪メーターの水平線は上端から 72px） */
    --glow-y: calc(var(--top-h) + 58 * var(--u));
  }

  [data-mode="note"] .main {
    --glow-y: calc(var(--top-h) + 20 * var(--u));
  }

  /* 空：--sky-1 から --sky-2 へのグラデーションに、太陽の位置に合わせてうっすら光を差す */
  .sky {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: calc(240 * var(--u));
    pointer-events: none;
    background:
      radial-gradient(
        calc(420 * var(--u)) calc(150 * var(--u)) at
          calc(var(--pad) + (100% - 2 * var(--pad)) * var(--sun-t, 0.8)) var(--glow-y),
        var(--sun-glow),
        transparent 72%
      ),
      linear-gradient(180deg, var(--sky-1) 0%, var(--sky-2) 52%, var(--bg) 100%);
  }

  [data-mode="note"] .sky {
    height: calc(150 * var(--u));
  }

  .body {
    position: relative;
    flex: 1;
    min-height: 0;
  }
</style>
