// 振り返り・ノートのエディタ（CodeMirror 6。アーキテクチャ設計書 3章）に共通の動き。見た目とキーは
// 部品（src/board/ReflectionEditor.svelte・src/note/NoteView.svelte）が `extensions` で足す。
//
// - 入力は自動保存（DocSaver）で送る。エージェントの変更（changed で読み込み直した最新）は、まだ保存して
//   いない入力を残したまま差分として取り込み、カーソルを保つ。取り込みは取り消し（Mod+Z）の対象に
//   しない。IME の変換中は、変換が終わるまで取り込みを待つ（docs/design/01-architecture.md 4.1）。
// - 入力中の状態（フォーカス・カーソル行・変換中・最後の入力の時刻）は `EditorPresence` に書き、
//   変わるたびに `onActivity` を呼ぶ（画面全体の `InputTracker` が知らせ直す）。
// - 日を移ったら、前の日の保存は片付け（保存し終えるまで再試行を続ける）、その日の本文を読み直す。
//   読み終えるまでは入力を受け付けない（読む前の入力を失わないため）。

import { history, historyKeymap, standardKeymap } from "@codemirror/commands";
import { Annotation, EditorState, Transaction, type Extension } from "@codemirror/state";
import { EditorView, keymap, type KeyBinding, type ViewUpdate } from "@codemirror/view";
import { DocSaver, type SaveStatus } from "./autosave.svelte";
import type { LogicalDate, Snapshot, TextDoc } from "./bindings";
import { core } from "./core";
import type { EditorPresence } from "./ui_report.svelte";

/** 読めなかったときに読み直すまでの間（ミリ秒）。 */
const RETRY_MS = 1000;

export interface TextDocEditorOptions {
  doc: TextDoc;
  save: SaveStatus;
  presence: EditorPresence;
  onActivity: () => void;
  /** エディタの見た目と、部品が受け持つキー（標準のキーより先に試す）。状態を作るたびに呼ぶ。 */
  extensions: () => { extensions: Extension[]; keys: readonly KeyBinding[]; label: string };
}

/** 本文 `cur` を `next` にする、変わった所だけの置き換え（前後の同じ部分を除く）。 */
export function diffChange(cur: string, next: string): { from: number; to: number; insert: string } | null {
  if (cur === next) return null;
  let head = 0;
  while (head < cur.length && head < next.length && cur[head] === next[head]) head++;
  let tail = 0;
  while (
    tail < cur.length - head &&
    tail < next.length - head &&
    cur[cur.length - 1 - tail] === next[next.length - 1 - tail]
  ) {
    tail++;
  }
  return { from: head, to: cur.length - tail, insert: next.slice(head, next.length - tail) };
}

/** 選択範囲のある行（1 始まり。選択範囲が複数行にまたがるときはその全部）。 */
export function cursorLines(state: EditorState): number[] {
  const out = new Set<number>();
  for (const r of state.selection.ranges) {
    const a = state.doc.lineAt(r.from).number;
    const b = state.doc.lineAt(r.to).number;
    for (let n = a; n <= b; n++) out.add(n);
  }
  return [...out];
}

export class TextDocEditor {
  view: EditorView | null = null;
  readonly #o: TextDocEditorOptions;
  /** 取り込み（エージェントの変更）の印。この変更では入力として扱わない。 */
  readonly #remote = Annotation.define<boolean>();
  /** 変換が終わるまで待っている最新。 */
  #deferred: Snapshot | null = null;
  /** 読み込んでいる日（読み終える前に別の日へ移ったら、古い結果を捨てる）。 */
  #loading: LogicalDate | null = null;

  constructor(o: TextDocEditorOptions) {
    this.#o = o;
  }

  /** エディタを `parent` に作る（まだ本文は無く、入力を受け付けない）。 */
  mount(parent: HTMLElement): EditorView {
    this.view = new EditorView({ parent, state: this.#stateOf("", false) });
    return this.view;
  }

  /** 使うのをやめる。表示している日の保存は片付ける（保存し終えるまで再試行を続ける）。 */
  destroy() {
    this.#loading = null;
    this.#closeSaver();
    this.view?.destroy();
    this.view = null;
  }

  /** `editable` が偽なら入力を受け付けない（その日の本文を読み終えるまで）。 */
  #stateOf(body: string, editable: boolean): EditorState {
    const p = this.#o.presence;
    const own = this.#o.extensions();
    return EditorState.create({
      doc: body,
      extensions: [
        EditorView.editable.of(editable),
        history(),
        keymap.of([...own.keys, ...historyKeymap, ...standardKeymap]),
        EditorView.lineWrapping,
        ...own.extensions,
        EditorView.contentAttributes.of({ "aria-label": own.label, spellcheck: "false" }),
        EditorView.updateListener.of((u) => this.#onUpdate(u)),
        EditorView.domEventHandlers({
          compositionstart: () => {
            p.composing = true;
            this.#o.onActivity();
          },
          compositionend: () => {
            p.composing = false;
            p.lastInputAt = Date.now();
            // 変換が終わってから、待っていた取り込みを行う（CodeMirror が変換の結果を入れた後）
            setTimeout(() => {
              if (this.#deferred !== null) this.#absorb(this.#deferred);
            }, 0);
            this.#o.onActivity();
          },
        }),
      ],
    });
  }

  #onUpdate(u: ViewUpdate) {
    const p = this.#o.presence;
    if (u.docChanged && !u.transactions.some((tr) => tr.annotation(this.#remote))) {
      p.lastInputAt = Date.now();
      p.saver?.edited();
    }
    if (u.focusChanged) {
      p.focused = u.view.hasFocus;
      // フォーカスが外れたら、まだ保存していない入力をすぐ送る
      if (!u.view.hasFocus) void p.saver?.flush();
    }
    if (u.docChanged || u.selectionSet || u.focusChanged) {
      p.cursorLines = cursorLines(u.state);
      this.#o.onActivity();
    }
  }

  /** 本文を `text` に置き換える（取り込み）。変わった所だけを置き換え、カーソルと選択範囲を保つ。 */
  #replace(text: string) {
    const v = this.view;
    if (v === null) return;
    const changes = diffChange(v.state.doc.toString(), text);
    if (changes === null) return;
    v.dispatch({ changes, annotations: [this.#remote.of(true), Transaction.addToHistory.of(false)] });
  }

  #absorb(latest: Snapshot) {
    if (this.view?.composing) {
      this.#deferred = latest;
      return;
    }
    this.#deferred = null;
    this.#o.presence.saver?.absorb(latest);
  }

  #closeSaver() {
    const p = this.#o.presence;
    const prev = p.saver;
    p.saver = null;
    if (prev === null) return;
    // close はその時点の本文を写すので、エディタを空にする前に呼ぶ
    p.closing.add(prev);
    void prev.close();
    // 保存し終えるまで再試行していても、終えたら除く
    void prev.whenClosed().then(() => p.closing.delete(prev));
  }

  /** 日 `d` の本文を読み、エディタをその本文で始める（前の日の保存は片付ける）。 */
  async open(d: LogicalDate) {
    const p = this.#o.presence;
    this.#loading = d;
    this.#deferred = null;
    this.#closeSaver();
    this.view?.setState(this.#stateOf("", false));
    // 読み終えるまでは前の日のカーソル行や見出しを残さない
    p.cursorLines = [];
    this.#o.onActivity();
    let snap: Snapshot | null = null;
    while (snap === null && this.#loading === d) {
      try {
        snap = await core.getTextDoc(this.#o.doc, d);
      } catch {
        // 読めなければ間を空けて読み直す（読めるまでは入力を受け付けない）
        await new Promise((r) => setTimeout(r, RETRY_MS));
      }
    }
    const v = this.view;
    if (snap === null || this.#loading !== d || v === null) return;
    v.setState(this.#stateOf(snap.body, true));
    p.text = () => this.view?.state.doc.toString() ?? "";
    p.saver = new DocSaver(this.#o.save, this.#o.doc, d, snap, {
      text: () => this.view?.state.doc.toString() ?? "",
      replace: (t) => this.#replace(t),
    });
    p.cursorLines = cursorLines(v.state);
    this.#o.onActivity();
  }

  /**
   * コアの版が `rev` に進んだ（ボードを読み込み直したとき）：最新を読み直して取り込む。日 `d` を
   * 開いていないときと、最後に保存した（取り込んだ）版より新しくないときは何もしない（項目だけが
   * 変わったときに、本文を読み直さないため）。
   */
  refresh(d: LogicalDate, rev: number) {
    const s = this.#o.presence.saver;
    if (s === null || s.date !== d || rev <= s.saved.rev) return;
    void core
      .getTextDoc(this.#o.doc, d)
      .then((latest) => {
        if (this.#o.presence.saver === s) this.#absorb(latest);
      })
      .catch(() => {});
  }
}
