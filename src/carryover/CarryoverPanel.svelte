<script lang="ts">
  // 持ち越しの画面（S-03。画面設計書 5章）。整理モードの上に重ねて出す（背後は --scrim で覆ってぼかし、
  // App.svelte が外枠を inert にする）。状態と操作は src/lib/carryover.svelte.ts。
  //
  // キー（画面設計書 5章）：j/k または ↑/↓ で行を移る、Space でチェックの切り替え、Enter で
  // 「選んだものを持ち越す」、Esc で後回し。分割の子の入力欄では、文字・Enter（改行）はそのまま打て、
  // Esc でその行のチェック欄へ戻る。表示している間は、ほかの画面のキー（Mod+K・Mod+Z など）を受け付けない。
  // Tab は画面の中だけを回る。
  import { tick, untrack } from "svelte";
  import type { Id } from "../lib/bindings";
  import type { CarryRow, Carryover } from "../lib/carryover.svelte";
  import { shortDate } from "../lib/dates";
  import { t } from "../lib/i18n";
  import CarryIcon from "../shell/CarryIcon.svelte";

  let { carry }: { carry: Carryover } = $props();

  const m = $derived(t());
  const c = $derived(m.carryover);
  const todos = $derived(carry.rows.filter((r) => r.item.kind === "todo"));
  const blockers = $derived(carry.rows.filter((r) => r.item.kind === "blocker"));
  /** 表示の順（TODO、障害）。 */
  const ordered = $derived([...todos, ...blockers]);

  /** 選んでいる行（読み直しで並びが変わっても、同じ項目に留まる）。 */
  let selId = $state<Id | null>(null);
  const selIndex = $derived(Math.max(0, ordered.findIndex((r) => r.item.id === selId)));

  let panel = $state<HTMLElement | null>(null);
  /** フォーカスを置いた一覧（前回の日。前回の日が変わったら置き直す）。開いていなければ `undefined`。 */
  let focusedFor: string | null | undefined = undefined;
  /** 選んでいる行の位置（読み直しで行が消えたとき、同じ位置へ移るため）。 */
  let lastIndex = 0;
  $effect(() => {
    const i = ordered.findIndex((r) => r.item.id === selId);
    if (i >= 0) lastIndex = i;
  });

  // 開いたら、一覧を読み終えたところで先頭の行（無ければ「選んだものを持ち越す」）へフォーカスを置く。
  // 閉じたら、開く直前にフォーカスのあった所（Carryover.returnFocus）へ戻す
  $effect(() => {
    if (!carry.open) {
      if (focusedFor !== undefined) {
        focusedFor = undefined;
        selId = null;
        lastIndex = 0;
        const back = carry.returnFocus;
        carry.returnFocus = null;
        void tick().then(() => {
          if (back instanceof HTMLElement && back.isConnected) back.focus();
        });
      }
      return;
    }
    if (!carry.loaded) return;
    const from = carry.from;
    if (focusedFor === from) return;
    focusedFor = from;
    void tick().then(() => focusRow(0));
  });

  // 読み直しで選んでいた行が消えたら、同じ位置の行へ移る（フォーカスも。消えたチェック欄にあった
  // フォーカスが背後へ落ちて、次のキーが効かなくならないように）
  $effect(() => {
    if (selId === null || ordered.some((r) => r.item.id === selId)) return;
    const n = Math.min(lastIndex, ordered.length - 1);
    selId = ordered[n]?.item.id ?? null;
    untrack(() => {
      if (panel !== null && !panel.contains(document.activeElement)) focusRow(n);
    });
  });

  function focusRow(i: number) {
    if (panel === null) return;
    if (ordered.length === 0) {
      panel.querySelector<HTMLElement>("[data-act='selected']")?.focus();
      return;
    }
    const n = Math.max(0, Math.min(ordered.length - 1, i));
    selId = ordered[n].item.id;
    void tick().then(() => {
      const box = panel?.querySelector<HTMLElement>(`input[data-carry-check="${ordered[n].item.id}"]`);
      box?.focus();
      box?.closest("li")?.scrollIntoView?.({ block: "nearest" });
    });
  }

  function rowIndexOf(target: EventTarget | null): number | null {
    const li = target instanceof Element ? target.closest<HTMLElement>("li[data-carry-row]") : null;
    if (!li) return null;
    const i = ordered.findIndex((r) => r.item.id === li.dataset.carryRow);
    return i < 0 ? null : i;
  }

  async function choose(row: CarryRow, choice: "split" | "drop") {
    carry.choose(row, choice);
    selId = row.item.id;
    await tick();
    if (row.choice === "split") panel?.querySelector<HTMLElement>(`textarea[data-carry-split="${row.item.id}"]`)?.focus();
  }

  function trapTab(e: KeyboardEvent) {
    if (panel === null) return;
    const list = [...panel.querySelectorAll<HTMLElement>("input, button, textarea")].filter(
      (el) => !(el as HTMLButtonElement).disabled && el.tabIndex >= 0,
    );
    if (list.length === 0) return;
    const first = list[0];
    const last = list[list.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  function onKeydown(e: KeyboardEvent) {
    // ほかの画面のキー（window で受けるもの）へ渡さない。文字の入力などの既定の動きは止めない
    e.stopPropagation();
    if (e.isComposing || e.keyCode === 229) return;
    const target = e.target as HTMLElement;
    if (e.key === "Tab") {
      trapTab(e);
      return;
    }
    if (target.tagName === "TEXTAREA") {
      if (e.key === "Escape") {
        e.preventDefault();
        focusRow(rowIndexOf(target) ?? selIndex);
      }
      return;
    }
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    if (e.key === "Escape") {
      e.preventDefault();
      void carry.postpone();
      return;
    }
    // ボタンの上の Enter はそのボタンを押す。修飾キー付きの Enter（Mod+Enter など）では確定しない
    // （確定は Mod+Z で取り消せず、押し間違いを防ぐため。操作仕様書 2章「取り消し・やり直し」）
    if (e.key === "Enter" && plain && !e.shiftKey && target.tagName !== "BUTTON") {
      e.preventDefault();
      if (!e.repeat) void carry.confirm("selected");
      return;
    }
    const i = rowIndexOf(target) ?? selIndex;
    if (plain && !e.shiftKey && (e.key === "j" || e.key === "ArrowDown")) {
      e.preventDefault();
      focusRow(i + 1);
    } else if (plain && !e.shiftKey && (e.key === "k" || e.key === "ArrowUp")) {
      e.preventDefault();
      focusRow(i - 1);
    } else if (!plain && e.key !== "Shift") {
      // Mod+Z などの取り消しや、ほかの画面の操作をしない（既定の動きも止める）
      e.preventDefault();
    }
  }

  // 背後（外枠）にフォーカスがあるときのキーも、ほかの画面の操作にしない（覆いをクリックしてフォーカスが
  // 外れたときなど）。Esc（後回し）と Enter（選んだものを持ち越す）はそのまま効かせ、ほかは行へ戻す
  function onWindowKeydown(e: KeyboardEvent) {
    if (!carry.open || panel === null) return;
    if (e.target instanceof Node && panel.contains(e.target)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === "Escape") void carry.postpone();
    else if (e.key === "Enter" && !e.repeat && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey)
      void carry.confirm("selected");
    else focusRow(selIndex);
  }

  const reviewAt = $derived(carry.reviewThreshold);
  const confirmDisabled = $derived(carry.busy || !carry.loaded);
</script>

<svelte:window onkeydowncapture={onWindowKeydown} />

{#snippet row(r: CarryRow)}
  <li
    class="crow"
    class:off={!r.checked && r.choice === null}
    class:closing={r.choice !== null}
    data-carry-row={r.item.id}
  >
    <label class="cmain">
      <input
        type="checkbox"
        class="cbox"
        checked={r.checked}
        tabindex={r.item.id === ordered[selIndex]?.item.id ? 0 : -1}
        aria-label={r.item.text}
        data-carry-check={r.item.id}
        onfocus={() => (selId = r.item.id)}
        onchange={(e) => carry.toggle(r, (e.currentTarget as HTMLInputElement).checked)}
      />
      <span class="ctext">{r.item.text}</span>
      {#if r.choice !== null}
        <span class="cstate">{r.choice === "split" ? c.splitState : c.dropState}</span>
      {/if}
      {#if r.item.carryCount > 0}
        <span
          class="carry"
          class:warn={r.item.carryCount >= reviewAt}
          role="img"
          aria-label={m.carryTimes(r.item.carryCount)}
          title={m.carryTimes(r.item.carryCount)}
        >
          <CarryIcon />{r.item.carryCount}
        </span>
      {/if}
    </label>
    {#if r.review}
      <div class="review">
        <span class="msg">{c.review(r.item.carryCount)}</span>
        <button
          type="button"
          class="btn"
          aria-pressed={r.choice === "split"}
          data-carry-review="split"
          onclick={() => void choose(r, "split")}>{c.split}</button
        >
        <button
          type="button"
          class="btn"
          aria-pressed={r.choice === "drop"}
          data-carry-review="drop"
          onclick={() => void choose(r, "drop")}>{c.drop}</button
        >
      </div>
      {#if r.choice === "split"}
        <div class="split-edit">
          <textarea
            rows={Math.max(2, r.children.split("\n").length + 1)}
            placeholder={c.splitPlaceholder}
            aria-label={c.splitPlaceholder}
            data-carry-split={r.item.id}
            bind:value={r.children}
          ></textarea>
        </div>
      {/if}
    {/if}
  </li>
{/snippet}

{#if carry.open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="ov" onkeydown={onKeydown}>
    <div
      class="panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="carry-title"
      data-carryover={carry.from ?? ""}
      bind:this={panel}
    >
      <div class="ph">
        <div class="label">{m.labels.carryOver}</div>
        <h2 id="carry-title">{carry.from === null ? (carry.loaded ? c.nothing : c.loading) : c.title(shortDate(carry.from))}</h2>
        {#if carry.message !== null}
          <p class="message" role="status">{c.messages[carry.message]}</p>
        {/if}
      </div>
      <div class="body">
        {#if todos.length > 0}
          <h3 class="label">{m.labels.region.todo}</h3>
          <ul class="clist">
            {#each todos as r (r.item.id)}{@render row(r)}{/each}
          </ul>
        {/if}
        {#if blockers.length > 0}
          <h3 class="label">{m.labels.region.blockers}</h3>
          <ul class="clist">
            {#each blockers as r (r.item.id)}{@render row(r)}{/each}
          </ul>
        {/if}
        {#if carry.loaded && ordered.length === 0}
          <p class="empty">{c.empty}</p>
        {/if}
      </div>
      <div class="actions">
        <!-- 一覧を読み終えるまでは確定できない（後回しはできる） -->
        <button
          type="button"
          class="btn primary"
          data-act="selected"
          disabled={confirmDisabled}
          onclick={() => void carry.confirm("selected")}>{c.carrySelected}<kbd>Enter</kbd></button
        >
        <button type="button" class="btn" data-act="all" disabled={confirmDisabled} onclick={() => void carry.confirm("all")}
          >{c.carryAll}</button
        >
        <button type="button" class="btn" data-act="none" disabled={confirmDisabled} onclick={() => void carry.confirm("none")}
          >{c.carryNone}</button
        >
        <button
          type="button"
          class="btn quiet later"
          data-act="later"
          disabled={carry.busy}
          onclick={() => void carry.postpone()}>{c.later}<kbd>Esc</kbd></button
        >
      </div>
    </div>
  </div>
{/if}

<style>
  .ov {
    position: fixed;
    inset: 0;
    z-index: 10;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 12vh 16px 16px;
    background: var(--scrim);
    backdrop-filter: blur(3px);
  }

  .panel {
    width: min(calc(620 * var(--u)), 100%);
    max-height: 76vh;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: calc(14 * var(--u));
    box-shadow: var(--shadow-panel);
  }

  .label {
    font-size: var(--fs-label);
    font-weight: 600;
    letter-spacing: 0.14em;
    color: var(--text-muted);
  }

  .ph {
    padding: calc(20 * var(--u)) calc(24 * var(--u)) calc(6 * var(--u));
  }

  h2 {
    margin: 2px 0 0;
    font-size: calc(17 * var(--u));
    font-weight: 500;
  }

  .message {
    margin: calc(6 * var(--u)) 0 0;
    font-size: var(--fs-small);
    color: var(--warn);
  }

  .body {
    overflow: auto;
    padding-bottom: calc(8 * var(--u));
    scrollbar-width: thin;
    scrollbar-color: var(--rule) transparent;
  }

  h3 {
    margin: 0;
    padding: calc(12 * var(--u)) calc(24 * var(--u)) calc(4 * var(--u));
  }

  .empty {
    margin: 0;
    padding: calc(12 * var(--u)) calc(24 * var(--u));
    color: var(--text-muted);
  }

  .clist {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .crow {
    position: relative;
    padding: 0 calc(24 * var(--u));
  }

  .crow:focus-within {
    box-shadow: inset 2px 0 0 var(--accent-strong);
    background: var(--sel);
  }

  .cmain {
    display: flex;
    align-items: flex-start;
    gap: calc(12 * var(--u));
    min-height: var(--row-h);
    padding: calc((var(--row-h) - var(--fs-body) * var(--lh-body)) / 2) 0;
    cursor: pointer;
  }

  .ctext {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .crow.off .ctext {
    color: var(--text-muted);
  }

  .crow.closing .ctext {
    color: var(--done);
    text-decoration: line-through;
    text-decoration-thickness: 1px;
  }

  /* チェックは丸（画面設計書 5章） */
  .cbox {
    appearance: none;
    -webkit-appearance: none;
    margin: 0;
    margin-top: calc((var(--fs-body) * var(--lh-body) - 16 * var(--u)) / 2);
    flex: none;
    position: relative;
    width: calc(16 * var(--u));
    height: calc(16 * var(--u));
    border: 1.3px solid var(--ring);
    border-radius: 50%;
    background: transparent;
    cursor: pointer;
  }

  .cbox:checked {
    background: var(--accent);
    border-color: var(--accent);
  }

  .cbox:checked::after {
    content: "";
    position: absolute;
    left: 33%;
    top: 18%;
    width: 26%;
    height: 46%;
    border: solid var(--bg);
    border-width: 0 calc(1.8 * var(--u)) calc(1.8 * var(--u)) 0;
    transform: rotate(45deg);
  }

  .cbox:focus-visible {
    outline: 2px solid var(--accent-strong);
    outline-offset: 2px;
  }

  .carry,
  .cstate {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: var(--fs-meta);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    line-height: calc(var(--fs-body) * var(--lh-body));
  }

  .carry.warn {
    color: var(--warn);
    font-weight: 500;
  }

  .review {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: calc(8 * var(--u));
    padding: 0 0 calc(10 * var(--u)) calc(28 * var(--u));
    font-size: var(--fs-small);
    color: var(--warn);
  }

  .review .msg {
    margin-right: 4px;
  }

  .split-edit {
    padding: 0 0 calc(10 * var(--u)) calc(28 * var(--u));
  }

  .split-edit textarea {
    display: block;
    width: 100%;
    box-sizing: border-box;
    resize: none;
    font: inherit;
    line-height: var(--lh-body);
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
    padding: calc(6 * var(--u)) calc(10 * var(--u));
  }

  .split-edit textarea:focus {
    border-color: var(--accent-strong);
    outline: none;
  }

  .split-edit textarea::placeholder {
    color: var(--text-muted);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: calc(8 * var(--u));
    align-items: center;
    padding: calc(14 * var(--u)) calc(24 * var(--u)) calc(16 * var(--u));
    border-top: 1px solid var(--rule);
  }

  .btn {
    font-size: var(--fs-small);
    line-height: 1.4;
    white-space: nowrap;
    padding: calc(6 * var(--u)) calc(13 * var(--u));
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }

  .btn:hover:not(:disabled) {
    background: var(--hover);
  }

  .btn:disabled {
    color: var(--text-muted);
    cursor: default;
  }

  .btn.primary {
    color: var(--accent-strong);
    border-color: var(--accent-strong);
    font-weight: 600;
  }

  .btn[aria-pressed="true"] {
    color: var(--accent-strong);
    border-color: var(--accent-strong);
  }

  .btn.quiet {
    border-color: transparent;
    color: var(--text-muted);
  }

  .btn kbd {
    margin-left: 6px;
  }

  .actions .later {
    margin-left: auto;
  }
</style>
