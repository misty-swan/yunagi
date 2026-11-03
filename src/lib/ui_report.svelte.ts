// 画面の入力中の状態を `report_ui_state` でコアに知らせる中身（docs/design/01-architecture.md 4.1・4.3）。
//
// - 項目：入力中の欄（BoardModel.editingItems）。
// - ノート・振り返り：守る行。カーソル行（エディタにフォーカスがあり、アプリが前面にあり、最後の入力から
//   EDITING_MS 以内か IME の変換中）と、未保存の変更がある行（DocSaver.unsavedLines）を合わせ、最後に
//   保存した本文の行番号にして、その版（rev）を添える。日を移って片付けている途中（保存し終えるまで
//   再試行している）の振り返りも含める。
// - 中身が変わったときだけ送る。送るのに失敗したら、次に知らせ直すときに送り直す。

import type { DocSaver } from "./autosave.svelte";
import type { EditorLines, UiReport } from "./bindings";
import { EDITING_MS } from "./board.svelte";
import { core } from "./core";
import { baseLinesOf } from "./text_merge";

/** エディタ（振り返り）の、守る行を求めるための状態。エディタの部品が書く。 */
export class EditorPresence {
  /** 表示している日の保存。 */
  saver: DocSaver | null = null;
  /** 日を移って片付けている途中の保存（保存し終えたら除く）。 */
  readonly closing = new Set<DocSaver>();
  focused = false;
  composing = false;
  lastInputAt = 0;
  /** カーソル（選択範囲）のあるエディタの行（1 始まり）。 */
  cursorLines: number[] = [];
  /** エディタのいまの本文。 */
  text: () => string = () => "";

  /** 守る行（無ければ入れない）。 */
  editors(now: number, active: boolean): EditorLines[] {
    const out: EditorLines[] = [];
    for (const s of this.closing) {
      const lines = s.unsavedLines();
      if (lines.length > 0) out.push({ doc: s.doc, date: s.date, rev: s.saved.rev, lines });
    }
    const s = this.saver;
    if (s === null) return out;
    const lines = new Set(s.unsavedLines());
    if (this.focused && active && (this.composing || now - this.lastInputAt < EDITING_MS)) {
      for (const l of baseLinesOf(s.saved.body, this.text(), this.cursorLines)) lines.add(l);
    }
    if (lines.size > 0) {
      out.push({ doc: s.doc, date: s.date, rev: s.saved.rev, lines: [...lines].sort((a, b) => a - b) });
    }
    return out;
  }

  /** 入力中でなくなる時刻（それまでに知らせ直す）。無ければ `null`。 */
  editingUntil(): number | null {
    return this.focused && !this.composing ? this.lastInputAt + EDITING_MS : null;
  }
}

export type UiState = Omit<UiReport, "session" | "seq">;

/** 入力中の状態を、変わったときだけ知らせる。 */
export class UiReporter {
  readonly #compute: () => UiState;
  readonly #send: (s: UiState) => Promise<unknown>;
  /** 最後に送った（送っている）中身。 */
  #last = "";

  constructor(compute: () => UiState, send: (s: UiState) => Promise<unknown> = core.reportUiState) {
    this.#compute = compute;
    this.#send = send;
  }

  /** いまの中身を求め、前に送ったものと違えば送る。 */
  update() {
    const state = this.#compute();
    const key = JSON.stringify(state);
    if (key === this.#last) return;
    this.#last = key;
    this.#send(state).catch(() => {
      // 送れなかった（データベースを開いていないなど）。次に知らせ直すときに送り直す
      if (this.#last === key) this.#last = "";
    });
  }
}

