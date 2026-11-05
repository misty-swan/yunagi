<script lang="ts">
  // 「New version」の通知（画面設計書 3章）。サイドバー下端（`placement="side"`）と、サイドバーを閉じている
  // ときの上部のバーの右側（`placement="top"`）に出す。選ぶと更新内容と「更新して再起動」を表示する。
  // 更新内容は配布元の文なので、プレーンテキストとして出す（HTML として解釈しない）。
  import { t } from "../lib/i18n";
  import type { UpdateStore } from "../lib/update.svelte";

  let {
    update,
    placement,
  }: {
    update: UpdateStore;
    placement: "side" | "top";
  } = $props();

  const m = $derived(t());
  let open = $state(false);
  let wrap = $state<HTMLElement | null>(null);
  let button = $state<HTMLButtonElement | null>(null);
  let panel = $state<HTMLElement | null>(null);
  const uid = $props.id();
  const panelId = `${uid}-update`;

  const busy = $derived(update.state.state === "downloading" || update.state.state === "restarting");

  // 表示の外を押したら閉じる
  $effect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (e.target instanceof Node && wrap?.contains(e.target)) return;
      open = false;
    };
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
  });

  // 開いたら、表示へフォーカスを移す（読み上げで内容が伝わり、Esc が届くように）
  $effect(() => {
    if (open) panel?.focus();
  });

  // 通知が消えたら（無効にした・再起動する）、表示も閉じる
  $effect(() => {
    if (!update.visible) open = false;
  });

  function close() {
    open = false;
    button?.focus();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && open) {
      e.stopPropagation();
      close();
    }
  }

  function onFocusOut(e: FocusEvent) {
    const next = e.relatedTarget;
    if (next instanceof Node && e.currentTarget instanceof Node && e.currentTarget.contains(next)) return;
    if (next !== null) open = false;
  }
</script>

{#if update.visible}
  <div class="wrap {placement}" bind:this={wrap} role="presentation" onkeydown={onKey} onfocusout={onFocusOut}>
    <button
      type="button"
      class="tag"
      bind:this={button}
      aria-expanded={open}
      aria-controls={open ? panelId : undefined}
      onclick={() => (open = !open)}
    >
      <span class="dot" aria-hidden="true"></span>{m.update.label}
    </button>
    {#if open}
      <div class="pop" id={panelId} bind:this={panel} role="dialog" aria-label={m.update.dialog} tabindex="-1">
        {#if update.version !== null}<p class="head">{m.update.heading(update.version)}</p>{/if}
        {#if update.state.state === "available" || update.state.state === "failed"}
          <p class="notes">{update.notes ?? m.update.noNotes}</p>
        {/if}
        {#if update.progress !== undefined}
          <p class="muted" role="status">{m.update.downloading(update.progress)}</p>
        {:else if update.state.state === "restarting"}
          <p class="muted" role="status">{m.update.restarting}</p>
        {:else if update.state.state === "failed"}
          <p class="warn" role="status">{m.update.failed}</p>
        {/if}
        {#if update.refused}<p class="warn" role="status">{m.update.saveFailing}</p>{/if}
        {#if update.canInstall}<p class="muted">{m.update.hint}</p>{/if}
        <div class="actions">
          <button type="button" class="btn" onclick={close}>{m.update.close}</button>
          <button type="button" class="btn primary" disabled={busy} onclick={() => void update.install()}>
            {m.update.install}
          </button>
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .wrap {
    position: relative;
    flex: none;
  }

  .tag {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    border: 0;
    background: none;
    padding: 0;
    cursor: pointer;
    white-space: nowrap;
    font-size: calc(11.5 * var(--u));
    color: var(--accent-strong);
  }

  .side .tag {
    padding: 0 10px;
    line-height: 1.9;
  }

  .top .tag {
    font-size: var(--fs-meta);
  }

  .tag:hover {
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }

  .pop {
    position: absolute;
    z-index: 10;
    width: calc(300 * var(--u));
    padding: calc(14 * var(--u)) calc(16 * var(--u));
    border: 1px solid var(--rule);
    border-radius: calc(12 * var(--u));
    background: var(--surface);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-small);
    color: var(--text);
  }

  /* サイドバーの下端：上へ開く */
  .side .pop {
    left: 0;
    bottom: calc(100% + 8px);
  }

  /* 上部のバー：下へ開く */
  .top .pop {
    right: 0;
    top: calc(100% + 10px);
  }

  .pop p {
    margin: 0 0 8px;
  }

  .head {
    font-weight: 600;
  }

  .notes {
    max-height: calc(160 * var(--u));
    overflow: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .muted {
    color: var(--text-muted);
  }

  .warn {
    color: var(--warn);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .btn {
    padding: calc(4 * var(--u)) calc(12 * var(--u));
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
    background: transparent;
    color: inherit;
    font-size: var(--fs-small);
    font-weight: 600;
    cursor: pointer;
  }

  .btn:hover:not(:disabled) {
    background: var(--hover);
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .primary {
    border-color: var(--accent-strong);
    color: var(--accent-strong);
  }
</style>
