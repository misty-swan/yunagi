<script lang="ts">
  // ノートモードの本文（画面設計書 4章 S-02）：その日のノートのエディタ（CodeMirror 6）と、右の Outline。
  //
  // - 本文の幅は 680px で中央に置く。左に縦の細い線（時間軸）を引き、時刻付き見出しの位置に丸を置く。
  //   いまカーソルがある区間（その行より上で最も近い時刻付き見出しの区間）の丸は塗る。
  // - 時刻付き見出しは、時刻を大きく（26px・細字・夕陽色）、件名を 17px で続け、「## 」はごく薄く出す
  //   （デザインガイド 4章）。ほかの行（`#` の見出し・`**` などの強調も）は、書いたとおりの本文として、
  //   太さ 400 の直立で出す（2026-11-03 決定。デザインガイド 4章「ノートの等幅」の「太さ」）。
  // - Mod+Shift+H（ノートモードの間、エディタの外でも）で時刻付き見出しを入れる。「- 」の行の Enter で
  //   次の行も「- 」で始める（操作仕様書 2章「ノートの編集」）。
  // - 設定のノートの書体が等幅なら、同梱の等幅の書体を読み込み終えてから当てる（src/lib/fonts.svelte.ts）。
  //   起動時は、読み込み終えるまで（上限 FONT_WAIT_MS）ノートの文字を出さない。
  // - 保存・取り込み・入力中の状態・日の移動は、振り返りと共通（src/lib/doc_editor.ts）。
  import { RangeSetBuilder, StateField, type EditorState } from "@codemirror/state";
  import { Decoration, EditorView, type DecorationSet } from "@codemirror/view";
  import { onMount, tick, untrack } from "svelte";
  import type { SaveStatus } from "../lib/autosave.svelte";
  import type { LogicalDate } from "../lib/bindings";
  import { TextDocEditor } from "../lib/doc_editor";
  import { FONT_WAIT_MS, monoFont, type FontLoader } from "../lib/fonts.svelte";
  import { t } from "../lib/i18n";
  import { noteActionOf } from "../lib/keys";
  import { continueList, insertTimeHeading, timeHeadingOf, type Edit } from "../lib/note";
  import { prefs, type NoteFont } from "../lib/prefs.svelte";
  import type { EditorPresence } from "../lib/ui_report.svelte";

  let {
    date,
    save,
    presence,
    refresh,
    onActivity,
    visible,
    narrow,
    mac,
    font = monoFont,
    now = () => new Date(),
    view = $bindable(null),
  }: {
    /** 表示している日（ボードを読むまでは `null`）。 */
    date: LogicalDate | null;
    save: SaveStatus;
    presence: EditorPresence;
    /** 変わるたびに、コアの最新を読み直して取り込む（ボードを読み込み直したとき）。 */
    refresh: unknown;
    onActivity: () => void;
    /** ノートモードか（キーを受け持つか）。 */
    visible: boolean;
    /** ウィンドウ幅が 900px 未満か（Outline を出さない）。 */
    narrow: boolean;
    mac: boolean;
    /** 等幅の書体の読み込み（テストで差し替える）。 */
    font?: FontLoader;
    /** いまの時刻（時刻付き見出しに入れる。テストで差し替える）。 */
    now?: () => Date;
    view?: EditorView | null;
  } = $props();

  const m = $derived(t());
  let host = $state<HTMLElement | null>(null);

  // ---- 時刻付き見出しと Outline ----

  interface Heading {
    /** 行の番号（1 始まり）。 */
    line: number;
    /** 行の終わりの位置（Outline から移るときのカーソルの位置）。 */
    end: number;
    time: string;
    title: string;
  }

  /** 本文の時刻付き見出し（上から順）。 */
  function headingsOf(state: EditorState): Heading[] {
    const out: Heading[] = [];
    const doc = state.doc;
    for (let n = 1; n <= doc.lines; n++) {
      const line = doc.line(n);
      // 見出しは「## 」で始まる行だけなので、先に安く絞る
      if (!line.text.startsWith("## ")) continue;
      const h = timeHeadingOf(line.text);
      if (h !== null) out.push({ line: n, end: line.to, time: h.time, title: h.title });
    }
    return out;
  }

  /** カーソルのある区間の見出しの行（その行より上で最も近い見出し。無ければ 0）。 */
  function currentOf(state: EditorState, heads: readonly Heading[]): number {
    const at = state.doc.lineAt(state.selection.main.head).number;
    let cur = 0;
    for (const h of heads) {
      if (h.line > at) break;
      cur = h.line;
    }
    return cur;
  }

  const headLine = Decoration.line({ class: "cm-th" });
  const headLineCur = Decoration.line({ class: "cm-th cm-th-cur" });
  const mdMark = Decoration.mark({ class: "cm-md" });
  const timeMark = Decoration.mark({ class: "cm-tm" });

  interface Outline {
    heads: Heading[];
    cur: number;
    deco: DecorationSet;
  }

  function outlineOf(state: EditorState, prev: Outline | null, docChanged: boolean): Outline {
    const heads = prev !== null && !docChanged ? prev.heads : headingsOf(state);
    const cur = currentOf(state, heads);
    if (prev !== null && !docChanged && prev.cur === cur) return prev;
    const b = new RangeSetBuilder<Decoration>();
    for (const h of heads) {
      const line = state.doc.line(h.line);
      const th = timeHeadingOf(line.text)!;
      b.add(line.from, line.from, h.line === cur ? headLineCur : headLine);
      b.add(line.from, line.from + th.timeFrom, mdMark);
      b.add(line.from + th.timeFrom, line.from + th.timeTo, timeMark);
    }
    return { heads, cur, deco: b.finish() };
  }

  /** 見出しの一覧と飾り。本文かカーソルが変わったときだけ求め直す。 */
  const outlineField = StateField.define<Outline>({
    create: (state) => outlineOf(state, null, true),
    update: (value, tr) => (tr.docChanged || tr.selection ? outlineOf(tr.state, value, tr.docChanged) : value),
    provide: (f) => EditorView.decorations.from(f, (o) => o.deco),
  });

  let heads = $state.raw<Heading[]>([]);
  let cur = $state(0);

  /** Outline を、エディタの見出しの一覧に合わせる（入力・カーソルの移動・本文の読み込みのたびに呼ぶ）。 */
  function syncOutline(state: EditorState) {
    const o = state.field(outlineField, false);
    if (o === undefined) return;
    if (o.heads !== heads) heads = o.heads;
    cur = o.cur;
  }

  // ---- 編集 ----

  function apply(v: EditorView, e: Edit) {
    v.dispatch({
      changes: { from: e.from, to: e.to, insert: e.insert },
      selection: { anchor: e.caret },
      scrollIntoView: true,
      userEvent: "input",
    });
  }

  /** 時刻付き見出しを入れる（入力を受け付けていないときは何もしない）。 */
  function timeHeading(v: EditorView): boolean {
    if (!v.state.facet(EditorView.editable)) return false;
    const line = v.state.doc.lineAt(v.state.selection.main.head);
    apply(v, insertTimeHeading({ from: line.from, text: line.text }, now()));
    return true;
  }

  function enter(v: EditorView): boolean {
    const sel = v.state.selection;
    if (v.composing || sel.ranges.length > 1 || !sel.main.empty) return false;
    const line = v.state.doc.lineAt(sel.main.head);
    const e = continueList({ from: line.from, text: line.text }, sel.main.head - line.from);
    if (e === null) return false;
    apply(v, e);
    return true;
  }

  // svelte-ignore state_referenced_locally
  const editor = new TextDocEditor({
    doc: "note",
    save,
    presence,
    // 入力・カーソルの移動・本文を読み終えたときに呼ばれる
    onActivity: () => {
      if (view !== null) syncOutline(view.state);
      onActivity();
    },
    extensions: () => ({
      extensions: [outlineField],
      keys: [{ key: "Enter", run: enter }],
      label: t().noteLabel,
    }),
  });

  /** Outline の見出しへ移る（カーソルをその行の終わりに置き、エディタに戻る）。 */
  function jump(h: Heading) {
    const v = view;
    if (v === null) return;
    v.dispatch({ selection: { anchor: Math.min(h.end, v.state.doc.length) }, scrollIntoView: true });
    v.focus();
  }

  function onKeydown(e: KeyboardEvent) {
    if (!visible || e.defaultPrevented || view === null) return;
    if (noteActionOf(e, mac) !== "timeHeading") return;
    e.preventDefault();
    if (!view.hasFocus) view.focus();
    timeHeading(view);
  }

  // ---- 書体 ----

  /** ノートに当てている書体（等幅は読み込み終えてから当てる）。 */
  let applied = $state<NoteFont>("body");
  /** ノートの文字を出すか（起動時に等幅を待つ間だけ偽）。 */
  let ready = $state(false);

  $effect(() => {
    const wanted = prefs.noteFont;
    untrack(() => {
      if (wanted === "body") {
        setFont("body");
        ready = true;
        return;
      }
      if (font.status === "loaded") {
        setFont("mono");
        ready = true;
        return;
      }
      const timer = setTimeout(() => (ready = true), FONT_WAIT_MS);
      void font.load().then((ok) => {
        clearTimeout(timer);
        if (ok && prefs.noteFont === "mono") setFont("mono");
        ready = true;
      });
    });
  });

  /** 書体を変える。字の幅が変わり折り返しが変わるので、測り直して、カーソルの行を画面に残す。 */
  function setFont(next: NoteFont) {
    if (applied === next) return;
    applied = next;
    void tick().then(() => {
      const v = view;
      if (v === null) return;
      v.requestMeasure();
      v.dispatch({ effects: EditorView.scrollIntoView(v.state.selection.main.head, { y: "nearest" }) });
    });
  }

  onMount(() => {
    view = editor.mount(host!);
    syncOutline(view.state);
    return () => {
      editor.destroy();
      view = null;
    };
  });

  $effect(() => {
    const d = date;
    if (view !== null && d !== null) untrack(() => void editor.open(d));
  });

  // ボードを読み込み直したら、ノートの最新も読み直して取り込む
  $effect(() => {
    void refresh;
    const d = date;
    if (d !== null) untrack(() => editor.refresh(d));
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="note" data-font={applied} data-ready={ready ? "" : undefined} data-narrow={narrow ? "" : undefined}>
  <div class="wrap">
    <div class="note-ed" bind:this={host}></div>
    <nav class="outline" aria-label={m.labels.outline}>
      <div class="label">{m.labels.outline}</div>
      {#each heads as h (h.line)}
        <button type="button" class:cur={h.line === cur} aria-current={h.line === cur ? "location" : undefined} onclick={() => jump(h)}>
          <span class="ot">{h.time}</span>
          {#if h.title !== ""}<span class="tt">{h.title}</span>{/if}
        </button>
      {/each}
    </nav>
  </div>
</div>

<style>
  .note {
    height: 100%;
    overflow: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--rule) transparent;
  }

  .wrap {
    display: flex;
    justify-content: center;
    gap: calc(120 * var(--u));
    padding: calc(22 * var(--u)) var(--pad) 0;
    min-height: 100%;
  }

  .note-ed {
    width: calc(680 * var(--u));
    max-width: 100%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    cursor: text;
  }

  /* 起動時に等幅を待つ間は、ノートの文字を出さない（読み込み終えてから描く。デザインガイド 4章） */
  .note:not([data-ready]) .note-ed {
    visibility: hidden;
  }

  .note-ed :global(.cm-editor) {
    flex: 1;
    background: transparent;
    color: var(--text);
    font: inherit;
  }

  .note-ed :global(.cm-editor.cm-focused) {
    outline: none;
  }

  .note-ed :global(.cm-scroller) {
    font: inherit;
    line-height: var(--lh-note);
    overflow: visible;
  }

  /* 左の時間軸：縦の細い線 */
  .note-ed :global(.cm-content) {
    min-height: 100%;
    padding: 0 0 40vh calc(44 * var(--u));
    caret-color: var(--accent-strong);
    background: linear-gradient(var(--rule), var(--rule)) no-repeat calc(7 * var(--u)) calc(12 * var(--u)) /
      1px calc(100% - 12 * var(--u));
  }

  .note[data-font="mono"] .note-ed :global(.cm-content) {
    font-family: var(--font-mono);
  }

  /* 時刻付き見出しを除き、どの行も太さ 400 の直立（等幅の書体は Regular だけを同梱するため。06 4章） */
  .note-ed :global(.cm-line) {
    padding: 0;
    font-weight: 400;
    font-style: normal;
  }

  .note-ed :global(.cm-cursor) {
    border-left-color: var(--accent-strong);
  }

  .note-ed :global(.cm-selectionBackground),
  .note-ed :global(.cm-focused .cm-selectionBackground) {
    background: var(--sel);
  }

  /* 時刻付き見出し：件名 17px・500、時刻 26px・300・夕陽色。等幅でも本文の書体のまま（06 4章） */
  /* 上の間は padding で空ける（CodeMirror は行の高さに margin を数えないため） */
  .note-ed :global(.cm-th) {
    position: relative;
    padding-top: calc(18 * var(--u));
    font-family: var(--font);
    font-size: calc(17 * var(--u));
    font-weight: 500;
    line-height: 1.6;
  }

  .note-ed :global(.cm-th:first-child) {
    padding-top: 0;
  }

  /* 時間軸の丸。カーソルのある区間の丸は塗る */
  .note-ed :global(.cm-th::before) {
    content: "";
    position: absolute;
    left: calc(-44 * var(--u));
    top: calc(32 * var(--u));
    width: calc(15 * var(--u));
    height: calc(15 * var(--u));
    border-radius: 50%;
    border: 1.5px solid var(--accent);
    background: var(--bg);
  }

  .note-ed :global(.cm-th:first-child::before) {
    top: calc(14 * var(--u));
  }

  .note-ed :global(.cm-th-cur::before) {
    background: var(--accent);
  }

  .note-ed :global(.cm-th .cm-md) {
    font-size: calc(11 * var(--u));
    font-weight: 400;
    color: var(--text-faint);
  }

  .note-ed :global(.cm-th .cm-tm) {
    margin-right: 4px;
    font-size: calc(26 * var(--u));
    font-weight: 300;
    color: var(--accent-strong);
    font-variant-numeric: tabular-nums;
  }

  .outline {
    flex: none;
    width: calc(190 * var(--u));
    padding-top: calc(8 * var(--u));
    position: sticky;
    top: 0;
    align-self: flex-start;
  }

  .outline button {
    display: flex;
    gap: 10px;
    width: 100%;
    margin-top: 8px;
    padding: 2px 0 2px 10px;
    border: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
    font-size: var(--fs-small);
    color: var(--text-muted);
    box-shadow: inset 2px 0 0 var(--rule);
  }

  .outline button:hover {
    color: var(--text);
  }

  .outline button.cur {
    color: var(--text);
    box-shadow: inset 2px 0 0 var(--accent);
  }

  .outline .ot {
    flex: none;
    font-variant-numeric: tabular-nums;
  }

  .outline .tt {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* 狭いウィンドウ（900px 未満）では Outline を出さず、本文を広く使う（画面設計書 4章） */
  .note[data-narrow] .outline {
    display: none;
  }
</style>
