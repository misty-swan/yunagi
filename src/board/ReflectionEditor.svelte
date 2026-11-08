<script lang="ts">
  // 振り返りのエディタ（画面設計書 3章「各領域の表示」）。CodeMirror 6（アーキテクチャ設計書 3章）。
  //
  // - 「## 」で始まる行は見出し：夕陽色（`--accent-strong`）の小さな文字にし、「## 」はごく薄く出す。
  // - Mod+Shift+R で「良かったこと／詰まったこと／明日の一手」の見出しを入れる（操作仕様書 2章）。
  // - 保存・取り込み・入力中の状態・日の移動は、ノートと共通（src/lib/doc_editor.ts）。
  // - 設定の Vim モードが有効なら、エディタを Vim のキーで操作する（src/lib/editor_vim.ts。操作仕様書 4.2）。
  //   `<Space>t` `<Space>b` `<Space>h` は何もしない（ノートだけの操作）。ノーマルモードの Tab・Shift+Tab は、
  //   整理モードのリストと同じく領域を移る。現在のモードは右下に小さく出す。
  import { Prec, RangeSetBuilder } from "@codemirror/state";
  import { Decoration, EditorView, ViewPlugin, type DecorationSet, type ViewUpdate } from "@codemirror/view";
  import { onMount, untrack } from "svelte";
  import type { SaveStatus } from "../lib/autosave.svelte";
  import type { LogicalDate } from "../lib/bindings";
  import { selectMatch, TextDocEditor } from "../lib/doc_editor";
  import { t } from "../lib/i18n";
  import { boardActionOf } from "../lib/board_keys";
  import { vimIdle, vimModeOf, type VimMode } from "../lib/editor_vim";
  import { isMacPlatform } from "../lib/keys";
  import { blank } from "../lib/note";
  import { prefs } from "../lib/prefs.svelte";
  import type { EditorPresence } from "../lib/ui_report.svelte";
  import VimBadge from "../shell/VimBadge.svelte";

  let {
    date,
    save,
    presence,
    rev,
    onActivity,
    onRegion = () => {},
    find = null,
    onFound = () => {},
    view = $bindable(null),
  }: {
    date: LogicalDate;
    save: SaveStatus;
    presence: EditorPresence;
    /** ボードを読んだときの振り返りの版（進んだら、コアの最新を読み直して取り込む）。 */
    rev: number;
    onActivity: () => void;
    /** Vim のノーマルモードの Tab・Shift+Tab：前（-1）・次（+1）の領域へ移る。 */
    onRegion?: (dir: -1 | 1) => void;
    /** 検索パレットの結果から移る先：その日の振り返りで、検索語 `text` と一致した所（`n` は選ぶたびに変わる）。 */
    find?: { date: LogicalDate; text: string; n: number } | null;
    /** `find` へ移り終えた（一致する所が無ければ、先頭へ移る）。 */
    onFound?: () => void;
    view?: EditorView | null;
  } = $props();

  /** 本文を読み終えた日（`find` は、それを待ってから移る）。 */
  let loaded = $state<LogicalDate | null>(null);

  let host = $state<HTMLElement | null>(null);
  /** Vim の現在のモード（Vim が無効なら `null`）。 */
  let vimMode = $state<VimMode | null>(null);
  /** ノーマルモードの、続きのキーを待っている `]`・`[`（`]]`・`[[` の1打目。操作仕様書 4.3）。 */
  let regionPending: "]" | "[" | null = null;

  const headingLine = Decoration.line({ class: "cm-ref-h" });
  const headingMark = Decoration.mark({ class: "cm-md" });

  /** 「## 」で始まる行を見出しとして飾る。 */
  function headings(v: EditorView): DecorationSet {
    const b = new RangeSetBuilder<Decoration>();
    for (const { from, to } of v.visibleRanges) {
      for (let pos = from; pos <= to; ) {
        const line = v.state.doc.lineAt(pos);
        if (line.text.startsWith("## ")) {
          b.add(line.from, line.from, headingLine);
          b.add(line.from, line.from + 3, headingMark);
        }
        pos = line.to + 1;
      }
    }
    return b.finish();
  }

  const headingPlugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(v: EditorView) {
        this.decorations = headings(v);
      }
      update(u: ViewUpdate) {
        if (u.docChanged || u.viewportChanged) this.decorations = headings(u.view);
      }
    },
    { decorations: (p) => p.decorations },
  );

  /** テンプレートを入れる：いまの行が空ならその行に、そうでなければ次の行から。最初の「- 」の後ろへ移る。 */
  function insertTemplate(v: EditorView): boolean {
    const lines = t().reflectionTemplate;
    const head = v.state.selection.main.head;
    const line = v.state.doc.lineAt(head);
    const empty = blank(line.text);
    const insert = (empty ? "" : "\n") + lines.join("\n");
    const from = empty ? line.from : line.to;
    const caret = from + (empty ? 0 : 1) + lines[0].length + 1 + lines[1].length;
    v.dispatch({ changes: { from, to: line.to, insert }, selection: { anchor: caret }, scrollIntoView: true });
    return true;
  }

  // svelte-ignore state_referenced_locally
  const editor = new TextDocEditor({
    doc: "reflection",
    save,
    presence,
    onActivity: () => onActivity(),
    extensions: () => ({
      // テンプレートを入れるキーは、設定で変えた割当に従う（CodeMirror のキーに固定で結ばない）
      extensions: [
        headingPlugin,
        // Vim の拡張（Prec.high）より先に受ける：Vim は `]]` `[[` を自分の移動に使うため
        Prec.highest(
          EditorView.domEventHandlers({
            blur: () => {
              regionPending = null;
              return false;
            },
            // マウスでカーソルを動かしたら、続きのキー待ちは捨てる
            mousedown: () => {
              regionPending = null;
              return false;
            },
            keydown: (e, v) => {
              // Vim のノーマルモードの Tab・Shift+Tab と `]]`・`[[` は領域の移動（整理モードのリストと同じ）。
              // `]` `[` の1打目は続きのキー待ち（Vim は受けない）。2打目が違えば捨てて、そのキーを通常どおり扱う
              const pending = regionPending;
              if (!["Shift", "Control", "Alt", "Meta", "AltGraph"].includes(e.key)) regionPending = null;
              if (!e.ctrlKey && !e.altKey && !e.metaKey && !e.isComposing && e.keyCode !== 229 && vimModeOf(v) === "normal") {
                if (e.key === "Tab") {
                  onRegion(e.shiftKey ? -1 : 1);
                  return true;
                }
                // Vim が回数・オペレータ・文字（`f]` `r]` など）を待っている間は、Vim の入力なので奪わない
                if ((e.key === "]" || e.key === "[") && vimIdle(v)) {
                  e.preventDefault();
                  // 押し続けたときの繰り返しは、2打目に数えない
                  if (pending === e.key && !e.repeat) onRegion(e.key === "]" ? 1 : -1);
                  else regionPending = e.key;
                  return true;
                }
              }
              if (boardActionOf(e, isMacPlatform()) !== "template") return false;
              e.preventDefault();
              return insertTemplate(v);
            },
          }),
        ),
      ],
      keys: [],
    }),
    label: () => t().reflectionLabel,
    onVimMode: (m) => {
      vimMode = m;
      regionPending = null;
    },
    onLoaded: (d) => (loaded = d),
  });

  // 設定の Vim モードを、開いているエディタにすぐ反映する
  $effect(() => {
    const on = prefs.vimMode;
    untrack(() => editor.setVim(on));
  });

  // 検索パレットの結果から移る：その日の本文を読み終えてから、一致した所を選ぶ
  let lastFind = 0;
  $effect(() => {
    const f = find;
    const v = view;
    if (f === null || v === null || loaded !== f.date || date !== f.date || f.n === lastFind) return;
    lastFind = f.n;
    untrack(() => {
      selectMatch(v, f.text);
      onFound();
    });
  });

  // 言語を変えたら、読み上げ用の名前を付け直す
  $effect(() => {
    void t();
    untrack(() => editor.relabel());
  });

  onMount(() => {
    view = editor.mount(host!);
    return () => {
      editor.destroy();
      view = null;
    };
  });

  $effect(() => {
    const d = date;
    if (view === null) return;
    untrack(() => {
      // 読み終えるまでは、読んだ日にしない（同じ日を読み直すときも。読んでいる途中の空の本文へ `find` で
      // 移らないため）
      loaded = null;
      void editor.open(d);
    });
  });

  // ボードを読み込み直して振り返りの版が進んでいたら、最新を読み直して取り込む
  $effect(() => {
    const r = rev;
    const d = date;
    untrack(() => editor.refresh(d, r));
  });
</script>

<div class="ref-root">
  <div class="ref-ed" bind:this={host}></div>
  <!-- 文が長くて振り返りの領域がスクロールしても、見える範囲の右下に残す（高さ 0 の枠を下端に貼り付ける） -->
  <div class="ref-badge"><VimBadge mode={vimMode} /></div>
</div>

<style>
  /* Vim のモードの表示を右下に置くための外枠 */
  .ref-root {
    position: relative;
    flex: 1;
    display: flex;
    flex-direction: column;
  }

  .ref-badge {
    position: sticky;
    bottom: 0;
    flex: none;
    height: 0;
  }

  .ref-ed {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: calc(3 * var(--row-h));
    /* 項目の行（ItemRow）と同じ並び：左端の線は、本文の左の余白に置く */
    margin-left: calc(-14 * var(--u));
    padding-left: calc(14 * var(--u));
    cursor: text;
  }

  /* Windows の強制カラー（ハイコントラスト）は box-shadow を消すので、線がなくならないよう、トークンの色のまま描く
     （フォーム部品と同じ方針。06 11章） */
  @media (forced-colors: active) {
    .ref-ed {
      forced-color-adjust: none;
    }
  }

  /* フォーカスがあるとき：項目の行と同じ左端の 2px の線（06 6章）。Vim の有無によらない。
     面（--sel）は敷かない：選択範囲も --sel なので、重なると選択が見えなくなる */
  .ref-ed:focus-within {
    box-shadow: inset 2px 0 0 var(--accent-strong);
  }

  .ref-ed :global(.cm-editor) {
    flex: 1;
    background: transparent;
    color: var(--text);
    font: inherit;
  }

  /* CodeMirror 既定の枠は消す。代わりの表示は上の .ref-ed:focus-within */
  .ref-ed :global(.cm-editor.cm-focused) {
    outline: none;
  }

  .ref-ed :global(.cm-scroller) {
    font: inherit;
    line-height: var(--lh-body);
  }

  .ref-ed :global(.cm-content) {
    padding: 0 0 calc(16 * var(--u));
    caret-color: var(--accent-strong);
  }

  .ref-ed :global(.cm-line) {
    padding: 0;
  }

  .ref-ed :global(.cm-cursor) {
    border-left-color: var(--accent-strong);
  }

  .ref-ed :global(.cm-selectionBackground),
  .ref-ed :global(.cm-focused .cm-selectionBackground) {
    background: var(--sel);
  }

  /* 見出し：夕陽色の小さな文字。「## 」はごく薄く（画面設計書 3章） */
  .ref-ed :global(.cm-ref-h) {
    font-size: calc(12.5 * var(--u));
    font-weight: 500;
    letter-spacing: 0.08em;
    color: var(--accent-strong);
  }

  .ref-ed :global(.cm-md) {
    color: var(--text-faint);
    font-weight: 400;
  }
</style>
