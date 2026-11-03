<script lang="ts">
  // 上部のバー（画面設計書 3章「上部のバー」）。高さ 60px。
  // 左から「（サイドバーを閉じているときだけロゴ）」「‹ 日付 ›」「Today」「Board / Note」、
  // 右端に「短時間の知らせ」「検索欄」「◆ 未確認の変更」「保存状態」。
  import type { SaveStatus } from "../lib/autosave.svelte";
  import type { LogicalDate } from "../lib/bindings";
  import { monthDay, weekday } from "../lib/dates";
  import { t, type SaveCause } from "../lib/i18n";
  import { keyLabel } from "../lib/keys";
  import type { Mode } from "../lib/shell.svelte";
  import type { ShellCommand } from "./commands";
  import { untrack } from "svelte";
  import Logo from "./Logo.svelte";

  let {
    date,
    today,
    mode,
    showLogo,
    unseen,
    notice = null,
    save,
    mac,
    onPrev,
    onNext,
    onToday,
    onMode,
    onCommand,
    saveDetailRequest = 0,
  }: {
    date: LogicalDate | null;
    today: LogicalDate | null;
    mode: Mode;
    showLogo: boolean;
    unseen: number;
    /** 短時間だけ出す知らせ（持ち越し済みの項目を変えようとしたときなど）。無ければ `null`。 */
    notice?: string | null;
    save: SaveStatus;
    mac: boolean;
    onPrev: () => void;
    onNext: () => void;
    onToday: () => void;
    onMode: (mode: Mode) => void;
    onCommand: (c: ShellCommand) => void;
    /** 増えたら、保存できていない原因の表示を開く（パレットの「保存できていない理由を見る」）。 */
    saveDetailRequest?: number;
  } = $props();

  const m = $derived(t());
  const isToday = $derived(date !== null && date === today);
  /** 保存の失敗の原因の文（内部のエラーの文は出さない。画面設計書 3章）。 */
  const cause = $derived.by((): SaveCause | null => {
    const kind = save.error?.kind;
    return kind === "storage" || kind === "not_ready" || kind === "internal" ? kind : null;
  });
  let detailOpen = $state(false);
  let detail = $state<HTMLElement | null>(null);
  let failButton = $state<HTMLButtonElement | null>(null);
  const uid = $props.id();
  const detailId = `${uid}-save-detail`;

  let header = $state<HTMLElement | null>(null);
  let wrap = $state<HTMLElement | null>(null);

  // 保存できたら、原因の表示も閉じる。中にフォーカスがあったら（再試行を押して成功したとき）、
  // 消える部品と一緒にフォーカスを失わないよう、上部のバーへ移す
  $effect.pre(() => {
    if (save.failing) return;
    if (wrap?.contains(document.activeElement)) header?.focus();
    detailOpen = false;
  });

  // 原因の表示の外を押したら閉じる
  $effect(() => {
    if (!detailOpen) return;
    const onDown = (e: PointerEvent) => {
      if (e.target instanceof Node && wrap?.contains(e.target)) return;
      detailOpen = false;
    };
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
  });

  // パレットから開く（失敗していなければ何もしない）
  let lastDetailRequest = untrack(() => saveDetailRequest);
  $effect(() => {
    const r = saveDetailRequest;
    if (r === lastDetailRequest) return;
    lastDetailRequest = r;
    if (untrack(() => save.failing)) detailOpen = true;
  });

  // 開いたら、原因の表示へフォーカスを移す（読み上げで原因と再試行が伝わり、Esc が届くように）
  $effect(() => {
    if (detailOpen) detail?.focus();
  });

  function closeDetail() {
    detailOpen = false;
    failButton?.focus();
  }

  function onDetailKey(e: KeyboardEvent) {
    if (e.key === "Escape" && detailOpen) {
      e.stopPropagation();
      closeDetail();
    }
  }

  // 原因の表示の外へフォーカスが移ったら閉じる
  function onDetailFocusOut(e: FocusEvent) {
    const next = e.relatedTarget;
    if (next instanceof Node && e.currentTarget instanceof Node && e.currentTarget.contains(next)) return;
    if (next !== null) detailOpen = false;
  }
</script>

<header class="top" bind:this={header} tabindex="-1">
  {#if showLogo}<span class="top-logo"><Logo /></span>{/if}

  {#if date !== null}
    <div class="datenav">
      <button
        type="button"
        class="icon-btn"
        aria-label={m.withKey(m.prevDay, keyLabel("prevDay", mac))}
        onclick={onPrev}
      >
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M10 3.5 5.5 8l4.5 4.5" /></svg>
      </button>
      <span class="date">{monthDay(date)} <span class="wd">{weekday(date)}</span></span>
      <button
        type="button"
        class="icon-btn"
        aria-label={m.withKey(m.nextDay, keyLabel("nextDay", mac))}
        onclick={onNext}
      >
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6 3.5 10.5 8 6 12.5" /></svg>
      </button>
      {#if isToday}
        <span class="today-pill is-today">{m.labels.today}</span>
      {:else}
        <button
          type="button"
          class="today-pill go"
          aria-label={m.withKey(m.backToday, keyLabel("today", mac))}
          onclick={onToday}
        >
          {m.labels.today} <kbd>{keyLabel("today", mac)}</kbd>
        </button>
      {/if}
    </div>
  {/if}

  <div class="seg" role="group" aria-label={m.modeSwitch}>
    <button type="button" aria-pressed={mode === "board"} onclick={() => onMode("board")}>
      {m.labels.board}
    </button>
    <button type="button" aria-pressed={mode === "note"} onclick={() => onMode("note")}>
      {m.labels.note}
    </button>
  </div>

  <div class="fill"></div>

  <span class="notice" role="status">{notice ?? ""}</span>

  <button type="button" class="search" onclick={() => onCommand("palette")}>
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path d="M7 2.5a4.5 4.5 0 1 1 0 9 4.5 4.5 0 0 1 0-9zM10.4 10.4 13.5 13.5" />
    </svg>
    <span>{m.searchLabel}</span>
    <kbd>{keyLabel("palette", mac)}</kbd>
  </button>

  {#if unseen > 0}
    <button
      type="button"
      class="agent-ind"
      aria-label={m.unseenChanges(unseen)}
      title={m.unseenChanges(unseen)}
      onclick={() => onCommand("agents")}
    >
      <span class="mk-agent" aria-hidden="true"></span>
      {unseen}
    </button>
  {/if}

  {#if save.failing}
    <div
      class="save-fail-wrap"
      bind:this={wrap}
      role="presentation"
      onkeydown={onDetailKey}
      onfocusout={onDetailFocusOut}
    >
      <button
        type="button"
        class="save-fail"
        bind:this={failButton}
        aria-expanded={detailOpen}
        aria-controls={detailOpen ? detailId : undefined}
        onclick={() => (detailOpen = !detailOpen)}
      >
        {m.saveFailed}
      </button>
      {#if detailOpen}
        <div class="pop" id={detailId} bind:this={detail} role="dialog" aria-label={m.saveFailed} tabindex="-1">
          {#if cause}<p>{m.saveCause[cause]}</p>{/if}
          <p class="muted">{m.saveKeep}</p>
          <div class="actions">
            <button type="button" class="btn" onclick={() => save.retryNow()}>{m.retry}</button>
          </div>
        </div>
      {/if}
    </div>
  {:else if save.saving}
    <span class="save-dot" role="img" aria-label={m.saving} title={m.saving}></span>
  {/if}
</header>

<style>
  .top {
    position: relative;
    display: flex;
    align-items: center;
    gap: calc(14 * var(--u));
    height: var(--top-h);
    padding: 0 var(--pad);
    flex: none;
    min-width: 0;
  }

  .top:focus {
    outline: none;
  }

  .top-logo {
    flex: none;
    display: inline-flex;
    padding-right: 14px;
    border-right: 1px solid var(--rule);
  }

  .datenav {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: none;
  }

  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: calc(28 * var(--u));
    height: calc(28 * var(--u));
    border: 0;
    border-radius: calc(8 * var(--u));
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }

  .icon-btn:hover {
    background: var(--hover);
    color: var(--text);
  }

  .icon-btn svg {
    width: calc(14 * var(--u));
    height: calc(14 * var(--u));
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .date {
    display: inline-flex;
    gap: 8px;
    padding: 0 6px;
    font-size: var(--fs-body);
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .wd {
    font-weight: 400;
    color: var(--text-muted);
  }

  .today-pill {
    margin-left: 6px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: calc(11.5 * var(--u));
    font-weight: 600;
    line-height: calc(20 * var(--u));
    padding: 0 calc(9 * var(--u));
    border-radius: 999px;
    border: 1px solid var(--accent-strong);
    background: transparent;
    color: var(--accent-strong);
    white-space: nowrap;
  }

  .today-pill.go {
    cursor: pointer;
  }

  .today-pill.go:hover {
    background: var(--sel);
  }

  .today-pill kbd {
    font-size: calc(10.5 * var(--u));
  }

  .seg {
    flex: none;
    display: inline-flex;
    margin-left: calc(10 * var(--u));
    padding: 3px;
    border-radius: var(--radius);
    background: var(--hover);
  }

  .seg button {
    font-size: var(--fs-small);
    padding: calc(3 * var(--u)) calc(14 * var(--u));
    border: 0;
    border-radius: calc(7 * var(--u));
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }

  .seg button[aria-pressed="true"] {
    background: var(--surface);
    color: var(--text);
    font-weight: 600;
  }

  .fill {
    flex: 1;
    min-width: 0;
  }

  .notice {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .notice:empty {
    display: none;
  }

  .search {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    flex: 0 1 calc(270 * var(--u));
    min-width: calc(44 * var(--u));
    height: calc(34 * var(--u));
    padding: 0 10px 0 12px;
    border: 1px solid var(--rule);
    border-radius: var(--radius);
    background: var(--field);
    color: var(--text-muted);
    font-size: var(--fs-small);
    cursor: pointer;
  }

  .search svg {
    flex: none;
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
  }

  .search span {
    flex: 1;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .agent-ind {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: calc(34 * var(--u));
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--agent);
    font-size: calc(12.5 * var(--u));
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }

  .agent-ind:hover {
    background: var(--hover);
  }

  .mk-agent {
    flex: none;
    width: 6px;
    height: 6px;
    background: var(--agent);
    transform: rotate(45deg);
  }

  .save-dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-muted);
  }

  .save-fail-wrap {
    position: relative;
    flex: none;
  }

  .save-fail {
    border: 0;
    background: none;
    padding: 0;
    cursor: pointer;
    white-space: nowrap;
    font-size: var(--fs-meta);
    font-weight: 600;
    color: var(--warn);
  }

  .save-fail:hover {
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .pop {
    position: absolute;
    right: 0;
    top: calc(100% + 10px);
    z-index: 10;
    width: calc(300 * var(--u));
    padding: calc(14 * var(--u)) calc(16 * var(--u));
    border: 1px solid var(--rule);
    border-radius: calc(12 * var(--u));
    background: var(--surface);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-small);
  }

  .pop p {
    margin: 0 0 8px;
  }

  .muted {
    color: var(--text-muted);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
  }

  .btn {
    padding: calc(4 * var(--u)) calc(12 * var(--u));
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
    background: transparent;
    font-size: var(--fs-small);
    font-weight: 600;
    cursor: pointer;
  }

  .btn:hover {
    background: var(--hover);
  }
</style>
