<script lang="ts">
  // 振り返りのエディタ（画面設計書 3章「各領域の表示」）。CodeMirror 6（アーキテクチャ設計書 3章）。
  //
  // - 「## 」で始まる行は見出し：夕陽色（`--accent-strong`）の小さな文字にし、「## 」はごく薄く出す。
  // - Mod+Shift+R で「良かったこと／詰まったこと／明日の一手」の見出しを入れる（操作仕様書 2章）。
  // - 入力は自動保存（DocSaver）で送る。エージェントの変更（changed で読み込み直した最新）は、まだ保存して
  //   いない入力を残したまま差分として取り込み、カーソルを保つ。取り込みは取り消し（Mod+Z）の対象に
  //   しない。IME の変換中は、変換が終わるまで取り込みを待つ（docs/design/01-architecture.md 4.1）。
  // - 日を移ったら、前の日の保存は片付け（保存し終えるまで再試行を続ける）、その日の本文を読み直す。
  import { history, historyKeymap, standardKeymap } from "@codemirror/commands";
  import { Annotation, EditorState, RangeSetBuilder, Transaction } from "@codemirror/state";
  import { Decoration, EditorView, ViewPlugin, keymap, type DecorationSet, type ViewUpdate } from "@codemirror/view";
  import { onMount, untrack } from "svelte";
  import { DocSaver, type SaveStatus } from "../lib/autosave.svelte";
  import type { LogicalDate, Snapshot } from "../lib/bindings";
  import { core } from "../lib/core";
  import { t } from "../lib/i18n";
  import type { EditorPresence } from "../lib/ui_report.svelte";

  let {
    date,
    save,
    presence,
    refresh,
    onActivity,
    view = $bindable(null),
  }: {
    date: LogicalDate;
    save: SaveStatus;
    presence: EditorPresence;
    /** 変わるたびに、コアの最新を読み直して取り込む（ボードを読み込み直したとき）。 */
    refresh: unknown;
    onActivity: () => void;
    view?: EditorView | null;
  } = $props();

  const m = $derived(t());
  let host = $state<HTMLElement | null>(null);
  /** 取り込み（エージェントの変更）の印。この変更では入力として扱わない。 */
  const remote = Annotation.define<boolean>();
  /** 変換が終わるまで待っている最新。 */
  let deferred: Snapshot | null = null;
  /** 読み込んでいる日（読み終える前に別の日へ移ったら、古い結果を捨てる）。 */
  let loading: LogicalDate | null = null;

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
    const lines = m.reflectionTemplate;
    const head = v.state.selection.main.head;
    const line = v.state.doc.lineAt(head);
    const empty = line.text.trim() === "";
    const insert = (empty ? "" : "\n") + lines.join("\n");
    const from = empty ? line.from : line.to;
    const to = empty ? line.to : line.to;
    const caret = from + (empty ? 0 : 1) + lines[0].length + 1 + lines[1].length;
    v.dispatch({ changes: { from, to, insert }, selection: { anchor: caret }, scrollIntoView: true });
    return true;
  }

  function cursorLines(state: EditorState): number[] {
    const out = new Set<number>();
    for (const r of state.selection.ranges) {
      const a = state.doc.lineAt(r.from).number;
      const b = state.doc.lineAt(r.to).number;
      for (let n = a; n <= b; n++) out.add(n);
    }
    return [...out];
  }

  /** `editable` が偽なら入力を受け付けない（その日の本文を読み終えるまで。読む前の入力を失わないため）。 */
  function stateOf(body: string, editable: boolean): EditorState {
    return EditorState.create({
      doc: body,
      extensions: [
        EditorView.editable.of(editable),
        history(),
        keymap.of([{ key: "Mod-Shift-r", run: insertTemplate, preventDefault: true }, ...historyKeymap, ...standardKeymap]),
        EditorView.lineWrapping,
        headingPlugin,
        EditorView.contentAttributes.of({ "aria-label": m.reflectionLabel, spellcheck: "false" }),
        EditorView.updateListener.of(onUpdate),
        EditorView.domEventHandlers({
          compositionstart: () => {
            presence.composing = true;
            onActivity();
          },
          compositionend: () => {
            presence.composing = false;
            presence.lastInputAt = Date.now();
            // 変換が終わってから、待っていた取り込みを行う（CodeMirror が変換の結果を入れた後）
            setTimeout(() => {
              if (deferred !== null) absorb(deferred);
            }, 0);
            onActivity();
          },
        }),
      ],
    });
  }

  function onUpdate(u: ViewUpdate) {
    if (u.docChanged && !u.transactions.some((tr) => tr.annotation(remote))) {
      presence.lastInputAt = Date.now();
      presence.saver?.edited();
    }
    if (u.focusChanged) {
      presence.focused = u.view.hasFocus;
      // フォーカスが外れたら、まだ保存していない入力をすぐ送る
      if (!u.view.hasFocus) void presence.saver?.flush();
    }
    if (u.docChanged || u.selectionSet || u.focusChanged) {
      presence.cursorLines = cursorLines(u.state);
      onActivity();
    }
  }

  /** 本文を `text` に置き換える（取り込み）。変わった所だけを置き換え、カーソルと選択範囲を保つ。 */
  function replace(text: string) {
    const v = view;
    if (v === null) return;
    const cur = v.state.doc.toString();
    if (cur === text) return;
    let head = 0;
    while (head < cur.length && head < text.length && cur[head] === text[head]) head++;
    let tail = 0;
    while (
      tail < cur.length - head &&
      tail < text.length - head &&
      cur[cur.length - 1 - tail] === text[text.length - 1 - tail]
    ) {
      tail++;
    }
    v.dispatch({
      changes: { from: head, to: cur.length - tail, insert: text.slice(head, text.length - tail) },
      annotations: [remote.of(true), Transaction.addToHistory.of(false)],
    });
  }

  function absorb(latest: Snapshot) {
    if (view?.composing) {
      deferred = latest;
      return;
    }
    deferred = null;
    presence.saver?.absorb(latest);
  }

  /** 日 `d` の本文を読み、エディタをその本文で始める（前の日の保存は片付ける）。 */
  async function open(d: LogicalDate) {
    loading = d;
    // 前の日の保存を先に片付ける（close はその時点の本文を写すので、エディタを空にする前に呼ぶ）
    const prev = presence.saver;
    presence.saver = null;
    if (prev !== null) {
      presence.closing.add(prev);
      void prev.close().then((done) => {
        if (done) presence.closing.delete(prev);
      });
    }
    view?.setState(stateOf("", false));
    let snap: Snapshot | null = null;
    while (snap === null && loading === d) {
      try {
        snap = await core.getTextDoc("reflection", d);
      } catch {
        // 読めなければ間を空けて読み直す（読めるまでは入力を受け付けない）
        await new Promise((r) => setTimeout(r, 1000));
      }
    }
    if (snap === null || loading !== d || view === null) return;
    view.setState(stateOf(snap.body, true));
    presence.text = () => view?.state.doc.toString() ?? "";
    presence.saver = new DocSaver(save, "reflection", d, snap, {
      text: () => view?.state.doc.toString() ?? "",
      replace,
    });
    presence.cursorLines = cursorLines(view.state);
    onActivity();
  }

  onMount(() => {
    view = new EditorView({ parent: host!, state: stateOf("", false) });
    return () => {
      loading = null;
      const s = presence.saver;
      presence.saver = null;
      if (s !== null) {
        presence.closing.add(s);
        void s.close().then((done) => done && presence.closing.delete(s));
      }
      view?.destroy();
      view = null;
    };
  });

  $effect(() => {
    const d = date;
    if (view !== null) untrack(() => void open(d));
  });

  // ボードを読み込み直したら、振り返りの最新も読み直して取り込む
  $effect(() => {
    void refresh;
    const d = date;
    const s = presence.saver;
    if (s === null || s.date !== d) return;
    void core
      .getTextDoc("reflection", d)
      .then((latest) => {
        if (presence.saver === s) absorb(latest);
      })
      .catch(() => {});
  });
</script>

<div class="ref-ed" bind:this={host}></div>

<style>
  .ref-ed {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: calc(3 * var(--row-h));
    cursor: text;
  }

  .ref-ed :global(.cm-editor) {
    flex: 1;
    background: transparent;
    color: var(--text);
    font: inherit;
  }

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
