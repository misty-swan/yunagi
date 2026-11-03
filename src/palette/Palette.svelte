<script lang="ts">
  // 検索パレット（S-04。画面設計書 6章）。画面中央に入力欄と結果を重ねる（パネルの形は持ち越し S-03 と
  // 同じ）。状態と検索は src/lib/palette.svelte.ts、選んだものを行うのは App（`onRun`）。
  //
  // キー：↑/↓ で行を移る、Enter で選ぶ、Esc で閉じる、Mod+K でも閉じる。フォーカスは入力欄に置いたまま
  // にし（行はマウスでも選べる）、Tab もこの画面の外へ出さない。開いている間は、ほかの画面のキー
  // （ノートの Mod+Shift+H・骨組みの Mod+J など）を受け付けない。入力欄の中の編集のキー（Mod+V など）は
  // そのまま効く。
  import { flushSync, tick, type Snippet } from "svelte";
  import type { LogicalDate } from "../lib/bindings";
  import { longDate, shortDate } from "../lib/dates";
  import { MESSAGES, t } from "../lib/i18n";
  import { shellActionOf } from "../lib/keys";
  import {
    dateTarget,
    isItemHit,
    matchCommands,
    paletteCommands,
    SEARCH_LIMIT,
    type CommandContext,
    type Palette,
    type PaletteChoice,
  } from "../lib/palette.svelte";
  import { prefs } from "../lib/prefs.svelte";
  import type { Snippet as TextSnippet } from "../lib/search_match";

  let {
    palette,
    today,
    context,
    onRun,
  }: {
    palette: Palette;
    /** 今日の論理日付（相対的な日付の基準）。 */
    today: LogicalDate | null;
    context: CommandContext;
    onRun: (choice: PaletteChoice) => void;
  } = $props();

  const m = $derived(t());
  const p = $derived(m.palette);
  const other = $derived(MESSAGES[prefs.lang === "ja" ? "en" : "ja"]);

  const commands = $derived(paletteCommands(context, m, other));
  const shownCommands = $derived(matchCommands(commands, palette.query));
  const date = $derived(dateTarget(palette.query, today));
  const hasQuery = $derived(palette.query.trim() !== "");
  const views = $derived(hasQuery ? palette.views : []);
  /** 結果の下に出す状態。答えが届くまでは、前の答えがあればそれを出したまま、何も添えない。 */
  const resultState = $derived.by((): "failed" | "searching" | "empty" | "limited" | null => {
    if (!hasQuery) return null;
    if (palette.failed) return "failed";
    if (palette.viewsFor !== palette.query.trim()) return views.length === 0 ? "searching" : null;
    if (views.length === 0) return "empty";
    return views.length >= SEARCH_LIMIT ? "limited" : null;
  });

  type Row = { id: string; choice: PaletteChoice };
  /** 選べる行（表示の順。日付へ移動・コマンド・検索結果）。 */
  const rows = $derived.by((): Row[] => {
    const out: Row[] = [];
    if (date !== null) out.push({ id: `date-${date}`, choice: { kind: "date", date } });
    for (const c of shownCommands) out.push({ id: `cmd-${c.id}`, choice: { kind: "command", id: c.id } });
    views.forEach((v, i) =>
      out.push({ id: `hit-${i}-${v.hit.kind}-${v.hit.refId}`, choice: { kind: "hit", view: v, query: palette.viewsFor } }),
    );
    return out;
  });
  const indexOf = $derived(new Map(rows.map((r, i) => [r.id, i])));

  /** 選んでいる行（入力が変わると先頭に戻る）。 */
  let active = $state(0);
  let lastQuery = "";
  $effect.pre(() => {
    const q = palette.query;
    if (q !== lastQuery) {
      lastQuery = q;
      active = 0;
    }
  });
  const activeIndex = $derived(rows.length === 0 ? -1 : Math.min(active, rows.length - 1));
  const activeId = $derived(activeIndex < 0 ? undefined : `pal-${rows[activeIndex].id}`);

  let panel = $state<HTMLElement | null>(null);
  let input = $state<HTMLInputElement | null>(null);
  let list = $state<HTMLElement | null>(null);

  // 開いたら入力欄へ。閉じたら、開く直前にフォーカスのあった所へ戻す（選んだものを行うときは、
  // 戻してから行う。`choose`）
  let wasOpen = false;
  $effect(() => {
    if (palette.open) {
      wasOpen = true;
      void tick().then(() => input?.focus());
      return;
    }
    if (!wasOpen) return;
    wasOpen = false;
    // 閉じた後の描き直し（背後の inert を外す）を終えてから効果が走るので、そのまま戻せる。ほかへ
    // フォーカスが移っていれば（持ち越しの画面が開いたなど）戻さない
    const back = palette.returnFocus;
    palette.returnFocus = null;
    const a = document.activeElement;
    if (back instanceof HTMLElement && back.isConnected && (a === null || a === document.body)) back.focus();
  });

  // 選んでいる行を見える所へ
  $effect(() => {
    const id = activeId;
    if (id === undefined || list === null) return;
    void tick().then(() => document.getElementById(id)?.scrollIntoView?.({ block: "nearest" }));
  });

  function close() {
    palette.close();
  }

  /** 閉じて、開く直前の所へフォーカスを戻してから、選んだものを行う（行う先がフォーカスを移せるように）。 */
  function choose(i: number) {
    const row = rows[i];
    if (row === undefined) return;
    const back = palette.returnFocus;
    palette.returnFocus = null;
    palette.close();
    // 背後の inert を外してから戻す
    flushSync();
    if (back instanceof HTMLElement && back.isConnected) back.focus();
    onRun(row.choice);
  }

  function move(delta: number) {
    if (rows.length === 0) return;
    active = (activeIndex + delta + rows.length) % rows.length;
  }

  function onKeydown(e: KeyboardEvent) {
    // ほかの画面のキー（window で受けるもの）へ渡さない。入力欄の編集の既定の動きは止めない
    e.stopPropagation();
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === "Tab") {
      // 入力欄の外へ出さない（行はフォーカスを持たない）
      e.preventDefault();
      input?.focus();
      return;
    }
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.key === "ArrowDown" && plain) {
      e.preventDefault();
      move(1);
    } else if (e.key === "ArrowUp" && plain) {
      e.preventDefault();
      move(-1);
    } else if (e.key === "Enter" && plain && !e.shiftKey) {
      e.preventDefault();
      if (!e.repeat && activeIndex >= 0) choose(activeIndex);
    } else {
      const action = shellActionOf(e, context.mac);
      if (action === null) return;
      // 骨組みの操作はしない。Mod+K はもう一度押すと閉じる
      e.preventDefault();
      if (action === "palette" && !e.repeat) close();
    }
  }

  // 背後にフォーカスがあるときのキーも、ほかの画面の操作にしない（覆いの外へフォーカスが落ちたとき）。
  // Esc は閉じ、ほかは入力欄へ戻す
  function onWindowKeydown(e: KeyboardEvent) {
    if (!palette.open || panel === null) return;
    if (e.target instanceof Node && panel.contains(e.target)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === "Escape") close();
    else input?.focus();
  }

  function onInput(e: Event) {
    const el = e.currentTarget as HTMLInputElement;
    palette.input(el.value, (e as InputEvent).isComposing === true);
  }

  function onCompositionEnd(e: CompositionEvent) {
    palette.input((e.currentTarget as HTMLInputElement).value);
  }

  /** 結果の日付（年を含む）と、持ち越し先（同じ年なら年を省く）。 */
  function carriedLabel(from: LogicalDate, to: LogicalDate): string {
    return p.carriedTo(from.slice(0, 4) === to.slice(0, 4) ? shortDate(to) : longDate(to));
  }
</script>

<svelte:window onkeydowncapture={onWindowKeydown} />

{#snippet marked(s: TextSnippet)}
  {#if s.cutStart}…{/if}{#if s.mark === null}{s.text}{:else}{s.text.slice(0, s.mark.from)}<mark>{s.text.slice(
        s.mark.from,
        s.mark.to,
      )}</mark>{s.text.slice(s.mark.to)}{/if}{#if s.cutEnd}…{/if}
{/snippet}

{#snippet option(id: string, content: Snippet, extra = "")}
  {@const i = indexOf.get(id) ?? -1}
  <!-- キーは入力欄（combobox）で受ける。行はマウスで選ぶためだけに押せる -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <li
    id={`pal-${id}`}
    class={`opt ${extra}`}
    role="option"
    aria-selected={i === activeIndex}
    data-palette-row={id}
    onmousedown={(e) => e.preventDefault()}
    onmousemove={() => {
      if (i !== activeIndex) active = i;
    }}
    onclick={() => choose(i)}
  >
    {@render content()}
  </li>
{/snippet}

{#if palette.open}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="ov" onkeydown={onKeydown} onmousedown={(e) => e.target === e.currentTarget && close()}>
    <div class="panel" role="dialog" aria-modal="true" aria-label={p.label} data-palette bind:this={panel}>
      <div class="field">
        <svg viewBox="0 0 16 16" aria-hidden="true">
          <path d="M7 2.5a4.5 4.5 0 1 1 0 9 4.5 4.5 0 0 1 0-9zM10.4 10.4 13.5 13.5" />
        </svg>
        <input
          type="text"
          role="combobox"
          aria-label={p.label}
          aria-expanded={rows.length > 0}
          aria-controls="pal-list"
          aria-activedescendant={activeId}
          aria-autocomplete="list"
          autocomplete="off"
          spellcheck="false"
          placeholder={p.placeholder}
          value={palette.query}
          oninput={onInput}
          oncompositionend={onCompositionEnd}
          bind:this={input}
        />
        <kbd>Esc</kbd>
      </div>
      <div class="body" bind:this={list}>
        <ul id="pal-list" role="listbox" aria-label={p.label}>
          {#if date !== null}
            <li class="group label" role="presentation">{m.labels.palette.goToDate}</li>
            {#snippet dateRow()}
              <span class="date">{longDate(date!)}</span>
              <span class="muted">{p.goTo}</span>
            {/snippet}
            {@render option(`date-${date}`, dateRow)}
          {/if}
          {#if shownCommands.length > 0}
            <li class="group label" role="presentation">{m.labels.palette.commands}</li>
            {#each shownCommands as c (c.id)}
              {#snippet cmdRow()}
                <span class="text">{c.label}</span>
                {#if c.active}<span class="inuse">{p.inUse}</span>{/if}
                {#if c.key !== null}<kbd>{c.key}</kbd>{/if}
              {/snippet}
              {@render option(`cmd-${c.id}`, cmdRow)}
            {/each}
          {/if}
          {#if hasQuery}
            <li class="group label" role="presentation">{m.labels.palette.results}</li>
            {#each views as v, i (`${i}-${v.hit.kind}-${v.hit.refId}`)}
              {#snippet hitRow()}
                <span class="date">{longDate(v.hit.date)}</span>
                <span class="kind label">{m.labels.palette.kind[v.hit.kind]}</span>
                <span class="text">
                  {@render marked(v.body)}
                  {#if v.note !== null}
                    <span class="note"><span class="muted">{p.resolutionNote}</span> {@render marked(v.note)}</span>
                  {/if}
                </span>
                {#if isItemHit(v.hit) && v.hit.carriedTo !== null}
                  <span class="carried">{carriedLabel(v.hit.date, v.hit.carriedTo.date)}</span>
                {/if}
              {/snippet}
              {@render option(`hit-${i}-${v.hit.kind}-${v.hit.refId}`, hitRow, "hit")}
            {/each}
          {/if}
        </ul>
        <!-- 検索の状態（読み上げにも伝える） -->
        <p class="state" class:warn={resultState === "failed"} class:none={resultState === null} role="status">
          {resultState === "failed"
            ? p.failed
            : resultState === "searching"
              ? p.searching
              : resultState === "empty"
                ? p.empty
                : resultState === "limited"
                  ? p.limited(SEARCH_LIMIT)
                  : ""}
        </p>
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

  /* パネルの形は持ち越し（S-03）と同じ */
  .panel {
    width: min(calc(620 * var(--u)), 100%);
    max-height: 76vh;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: calc(14 * var(--u));
    box-shadow: var(--shadow-panel);
    overflow: hidden;
  }

  .field {
    display: flex;
    align-items: center;
    gap: calc(10 * var(--u));
    padding: calc(14 * var(--u)) calc(20 * var(--u));
    border-bottom: 1px solid var(--rule);
  }

  .field svg {
    flex: none;
    width: calc(16 * var(--u));
    height: calc(16 * var(--u));
    fill: none;
    stroke: var(--text-muted);
    stroke-width: 1.4;
    stroke-linecap: round;
  }

  .field input {
    flex: 1;
    min-width: 0;
    font: inherit;
    font-size: calc(16 * var(--u));
    color: var(--text);
    background: transparent;
    border: 0;
    outline: none;
    padding: 0;
  }

  .field input::placeholder {
    color: var(--text-muted);
  }

  .body {
    overflow: auto;
    padding-bottom: calc(8 * var(--u));
    scrollbar-width: thin;
    scrollbar-color: var(--rule) transparent;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .label {
    font-size: var(--fs-label);
    font-weight: 600;
    letter-spacing: 0.14em;
    color: var(--text-muted);
  }

  .group {
    padding: calc(12 * var(--u)) calc(20 * var(--u)) calc(4 * var(--u));
  }

  .opt {
    display: flex;
    align-items: baseline;
    gap: calc(12 * var(--u));
    min-height: var(--row-h);
    padding: calc((var(--row-h) - var(--fs-body) * var(--lh-body)) / 2) calc(20 * var(--u));
    cursor: pointer;
    min-width: 0;
  }

  .opt[aria-selected="true"] {
    box-shadow: inset 2px 0 0 var(--accent-strong);
    background: var(--sel);
  }

  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hit .text {
    white-space: nowrap;
  }

  .note {
    margin-left: 0.6em;
  }

  .date {
    flex: none;
    font-variant-numeric: tabular-nums;
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .kind {
    flex: none;
    min-width: 6.5em;
  }

  .muted {
    color: var(--text-muted);
  }

  /* 一致した部分は太字の夕陽色（画面設計書 6章） */
  mark {
    background: transparent;
    color: var(--accent-strong);
    font-weight: 600;
  }

  .inuse,
  .carried {
    flex: none;
    font-size: var(--fs-meta);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .inuse {
    color: var(--accent-strong);
  }

  kbd {
    flex: none;
    font-family: inherit;
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .state {
    padding: calc(8 * var(--u)) calc(20 * var(--u));
    font-size: var(--fs-small);
    color: var(--text-muted);
  }

  .state.none {
    display: none;
  }

  .state.warn {
    color: var(--warn);
  }
</style>
