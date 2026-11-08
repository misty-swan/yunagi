<script lang="ts">
  // TODO・障害の領域（画面設計書 3章「各領域の表示」「空の状態」）：英字の見出しと件数（「6 left」は
  // `--accent-strong`、「3 open」は `--warn`）、項目の一覧。TODO が空なら「今日やることを書く（Enter）」
  // （今日以外は「やることを書く（Enter）」）だけを出す。障害が空なら、押すと足せる空の1行を置く。
  import type { CarriedTo, ItemKind, NoteSource } from "../lib/bindings";
  import type { BoardModel, FocusTarget } from "../lib/board.svelte";
  import { t } from "../lib/i18n";
  import { prefs } from "../lib/prefs.svelte";
  import ItemRow from "./ItemRow.svelte";

  let {
    kind,
    model,
    mac,
    isToday,
    reviewAt,
    focusReq,
    onFocus,
    onRegion,
    onOpenCarried,
    onOpenNoteSource,
    onActivity,
    region = $bindable(null),
  }: {
    kind: ItemKind;
    model: BoardModel;
    mac: boolean;
    isToday: boolean;
    reviewAt: number;
    focusReq: (FocusTarget & { n: number }) | null;
    onFocus: (target: FocusTarget | null) => void;
    /** 領域 `kind` から前（-1）・次（+1）の領域へ移る（Vim のノーマルモードの Tab）。 */
    onRegion: (kind: ItemKind, dir: -1 | 1) => void;
    onOpenCarried: (to: CarriedTo) => void;
    onOpenNoteSource: (source: NoteSource) => void;
    onActivity: () => void;
    region?: HTMLElement | null;
  } = $props();

  const m = $derived(t());
  const rows = $derived(kind === "todo" ? model.todos : model.blockers);
  const open = $derived(rows.filter((r) => r.status === "open").length);
  const headingId = $props.id();

  function add() {
    const target = model.add(kind, null);
    // Vim モードでは、足した行にそのまま書き始める（挿入モード）。足せなかったときは入らない
    if (target !== null && prefs.vimMode) model.insert = true;
    onFocus(target);
  }

  /** 領域の先頭（-1）・末尾（+1）の行へ移る（Vim の gg・G）。 */
  function edge(dir: -1 | 1) {
    const row = dir < 0 ? rows[0] : rows[rows.length - 1];
    if (row !== undefined) onFocus({ key: row.key, field: "text", offset: 0 });
  }

  /** 隣の行へ移る（↑ / ↓）。カーソルは同じ文字位置（本文より後ろなら末尾）。 */
  function neighbor(key: string, dir: -1 | 1, offset: number) {
    const i = rows.findIndex((r) => r.key === key);
    const next = rows[i + dir];
    if (next !== undefined) onFocus({ key: next.key, field: "text", offset });
  }

  /** 行を消す前に、隣の行へ移る（前へなら末尾に）。どちらにも無ければ領域にフォーカスを置く。 */
  function removing(key: string, dir: -1 | 1) {
    const i = rows.findIndex((r) => r.key === key);
    const next = rows[i + dir] ?? rows[i - dir];
    if (next !== undefined) onFocus({ key: next.key, field: "text", offset: next === rows[i + 1] ? 0 : "end" });
    else region?.focus();
  }

  // ---- ドラッグで並べ替え（操作仕様書 5章） ----

  let listEl = $state<HTMLElement | null>(null);
  /** ドラッグしている行と、落とす位置（動かした後の位置）と、位置の印の高さ。 */
  let drag = $state<{ key: string; to: number; y: number } | null>(null);

  /** 本文・チェック欄・印の上でなければ、行をつかむ（本文の上ではカーソルを置く）。 */
  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0 || listEl === null) return;
    const target = e.target as HTMLElement;
    const li = target.closest<HTMLElement>(".item");
    if (li === null || target.closest(".txt, .memo-edit, .memo, .chk, .carried-to") !== null) return;
    const key = li.dataset.key!;
    const y0 = e.clientY;
    let moved = false;
    const items = () => [...listEl!.querySelectorAll<HTMLElement>(":scope > .item")];
    const onMove = (ev: PointerEvent) => {
      if (!moved && Math.abs(ev.clientY - y0) < 5) return;
      moved = true;
      const els = items();
      const from = els.findIndex((el) => el.dataset.key === key);
      let pos = els.length;
      for (let k = 0; k < els.length; k++) {
        const r = els[k].getBoundingClientRect();
        if (ev.clientY < r.top + r.height / 2) {
          pos = k;
          break;
        }
      }
      const top = listEl!.getBoundingClientRect().top;
      const y = (els[pos]?.getBoundingClientRect().top ?? els.at(-1)?.getBoundingClientRect().bottom ?? top) - top;
      drag = { key, to: pos > from ? pos - 1 : pos, y };
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      const d = drag;
      drag = null;
      if (moved && d !== null) model.moveTo(d.key, d.to);
      onFocus({ key, field: "text", offset: "end" });
    };
    e.preventDefault();
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  }

  function onRegionKey(e: KeyboardEvent) {
    if (e.target !== region || e.isComposing || e.keyCode === 229) return;
    // Vim のノーマルモード：行が無い領域でも、行のあるときと同じに領域を移る（Tab・`]]`・`[[`。操作仕様書 4.3）
    if (prefs.vimMode && !model.insert && !e.ctrlKey && !e.altKey && !e.metaKey) {
      const pending = model.vimPending;
      if (!["Shift", "Control", "Alt", "Meta", "AltGraph"].includes(e.key)) model.vimPending = null;
      if (e.key === "Tab") {
        e.preventDefault();
        onRegion(kind, e.shiftKey ? -1 : 1);
        return;
      }
      if (e.key === "]" || e.key === "[") {
        e.preventDefault();
        if (pending === e.key) onRegion(kind, e.key === "]" ? 1 : -1);
        else model.vimPending = e.key;
        return;
      }
    }
    if (e.key === "Enter" && !e.altKey && !e.shiftKey && !e.metaKey && !e.ctrlKey) {
      e.preventDefault();
      add();
    }
  }
</script>

<!-- 空の領域にフォーカスがあるときの Enter で、項目を足す -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section
  bind:this={region}
  class="region"
  class:r-todo={kind === "todo"}
  class:r-blk={kind === "blocker"}
  data-region={kind}
  tabindex="-1"
  aria-labelledby={headingId}
  onkeydown={onRegionKey}
  onfocus={() => (model.vimPending = null)}
>
  <div class="rh">
    <h2 class="label" id={headingId}>{kind === "todo" ? m.labels.region.todo : m.labels.region.blockers}</h2>
    {#if kind === "todo"}
      <span class="rm">{m.labels.left(open)}</span>
    {:else if open > 0}
      <span class="rm warn">{m.labels.open(open)}</span>
    {/if}
  </div>
  <div class="scroll">
    {#if rows.length > 0}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <ul class="list" bind:this={listEl} onpointerdown={onPointerDown}>
        {#each rows as row (row.key)}
          <ItemRow
            {row}
            {model}
            {mac}
            {reviewAt}
            {focusReq}
            {onFocus}
            onNeighbor={neighbor}
            onRemoving={removing}
            onEdge={edge}
            onRegion={(dir) => onRegion(kind, dir)}
            {onOpenCarried}
            {onOpenNoteSource}
            {onActivity}
          />
        {/each}
        {#if drag !== null}
          <li class="drop-ind" aria-hidden="true" style:top="{drag.y - 1}px"></li>
        {/if}
      </ul>
    {:else if kind === "todo"}
      <button type="button" class="empty" tabindex="-1" onclick={add}>{m.emptyTodo(isToday)}</button>
    {:else}
      <button type="button" class="empty blank" tabindex="-1" aria-label={m.labels.region.blockers} onclick={add}
      ></button>
    {/if}
  </div>
</section>

<style>
  .region {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }

  .rh {
    display: flex;
    align-items: baseline;
    gap: 14px;
    margin: 0 0 calc(8 * var(--u));
    flex: none;
  }

  .rh h2 {
    margin: 0;
  }

  .rm {
    font-size: var(--fs-label);
    letter-spacing: 0.1em;
    color: var(--accent-strong);
  }

  .rm.warn {
    color: var(--warn);
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--rule) transparent;
    margin-left: calc(-14 * var(--u));
    padding-left: calc(14 * var(--u));
  }

  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    position: relative;
  }

  .drop-ind {
    position: absolute;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--accent-strong);
    pointer-events: none;
  }

  .empty {
    display: block;
    width: calc(100% + 14 * var(--u));
    margin: 0 0 0 calc(-14 * var(--u));
    padding: calc((var(--row-h) - var(--fs-body) * var(--lh-body)) / 2) calc(14 * var(--u));
    border: 0;
    border-radius: 0 var(--radius) var(--radius) 0;
    background: none;
    text-align: left;
    color: var(--text-muted);
    cursor: text;
  }

  .region:focus .empty {
    box-shadow: inset 2px 0 0 var(--accent-strong);
    background: var(--sel);
  }

  .empty.blank {
    min-height: var(--row-h);
  }
</style>
