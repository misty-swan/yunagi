// 画面の入力中の状態を `report_ui_state` でコアに知らせる中身（docs/design/01-architecture.md 4.1・4.3）。
//
// - 項目：入力中の欄（BoardModel.editingItems）。
// - ノート・振り返り：守る行。カーソル行（エディタにフォーカスがあり、アプリが前面にあり、最後の入力から
//   EDITING_MS 以内か IME の変換中）と、未保存の変更がある行（DocSaver.unsavedLines）を合わせ、最後に
//   保存した本文の行番号にして、その版（rev）を添える。日を移って片付けている途中（保存し終えるまで
//   再試行している）の振り返りも含める。
// - 中身が変わったときだけ送る。送るのに失敗したら、次に知らせ直すときに送り直す。
// - `report_ui_state` は毎回全体を置き換えるので、整理モード（項目・振り返り）とノートの分を、画面全体で
//   1つの `InputTracker` にまとめて送る（別々に送ると、互いの守る行を消し合う）。

import { untrack } from "svelte";
import type { DocSaver, SaveStatus } from "./autosave.svelte";
import type { EditingItem, EditorLines, UiReport } from "./bindings";
import { EDITING_MS } from "./board.svelte";
import { core } from "./core";
import { baseLinesOf } from "./text_merge";

/** エディタ（振り返り・ノート）の、守る行を求めるための状態。エディタの部品が書く。 */
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
      for (const l of this.#cursorBaseLines(s)) lines.add(l);
    }
    if (lines.size > 0) {
      out.push({ doc: s.doc, date: s.date, rev: s.saved.rev, lines: [...lines].sort((a, b) => a - b) });
    }
    return out;
  }

  /** カーソル行を、もとの本文の行番号にしたもの（入力やカーソルの移動のたびに呼ぶので、同じなら前の結果を使う）。 */
  #cursorBaseLines(s: DocSaver): number[] {
    // 差分は「未保存の変更がある行」と同じものを使う（1文字ごとに2回求めない）
    const { base, editor, hunks } = s.diff();
    const cursor = this.cursorLines.join(",");
    const c = this.#cache;
    if (c === null || c.base !== base || c.editor !== editor || c.cursor !== cursor) {
      this.#cache = { base, editor, cursor, lines: baseLinesOf(base, editor, this.cursorLines, hunks) };
    }
    return this.#cache!.lines;
  }

  #cache: { base: string; editor: string; cursor: string; lines: number[] } | null = null;

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


/** 項目の入力中の欄を返すもの（整理モードの `BoardModel`）。 */
export interface ItemsEditing {
  editingItems(now: number, active: boolean): EditingItem[];
  editingUntil(): number | null;
}

/**
 * 画面全体の入力中の状態（項目・振り返り・ノート）をまとめ、変わるたびに知らせる。アプリに1つだけ作り、
 * 整理モードの部品が `items`（項目）と `reflection`（振り返り）に、ノートモードの部品が `note` に自分の分を入れる。
 */
export class InputTracker {
  /** アプリが前面にあるか。 */
  active: boolean;
  /** 項目の入力中の欄（整理モードの本文を出すまでは無い）。 */
  items: ItemsEditing | null = null;
  readonly reflection = new EditorPresence();
  readonly note = new EditorPresence();
  readonly #reporter: UiReporter;
  #expiry: ReturnType<typeof setTimeout> | undefined;
  #stopped = false;
  #unwatch: () => void;

  /** `save` の保存の状態が変わったら（保存し終えて、まだ保存していない欄が減ったら）知らせ直す。 */
  constructor(
    save: SaveStatus,
    send?: (s: UiState) => Promise<unknown>,
    active = typeof document === "undefined" || document.hasFocus(),
  ) {
    this.active = active;
    this.#reporter = new UiReporter(() => {
      const now = Date.now();
      return {
        editingItems: this.items?.editingItems(now, this.active) ?? [],
        editors: [...this.reflection.editors(now, this.active), ...this.note.editors(now, this.active)],
      };
    }, send);
    // 送る部品を作ってから見張り始める
    this.#unwatch = $effect.root(() => {
      $effect(() => {
        void save.saving;
        void save.failing;
        untrack(this.activity);
      });
    });
  }

  /** 入力・フォーカス・保存の状態などが変わった：知らせ直し、入力中でなくなる時刻にもう一度知らせ直す。 */
  activity = () => {
    if (this.#stopped) return;
    this.#reporter.update();
    clearTimeout(this.#expiry);
    const now = Date.now();
    const until = [this.items?.editingUntil() ?? null, this.reflection.editingUntil(), this.note.editingUntil()];
    // すでに過ぎた時刻は待たない（入力中でなくなった後も、フォーカスがある間は過ぎた時刻が返るため）
    const next = Math.min(...until.filter((u): u is number => u !== null && u > now));
    if (Number.isFinite(next)) this.#expiry = setTimeout(this.activity, next - now + 1);
  };

  /** アプリが前面に来た・前面から外れた。 */
  setActive(active: boolean) {
    this.active = active;
    this.activity();
  }

  /** 知らせ直すのをやめる（画面を閉じたとき）。 */
  stop() {
    this.#stopped = true;
    this.#unwatch();
    clearTimeout(this.#expiry);
  }
}
