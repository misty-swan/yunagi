// 整理モードの TODO・障害の一覧（画面設計書 3章「各領域の表示」、操作仕様書 2章「一覧の編集」）。
// 見た目は持たない（src/board/ の部品がこれを呼ぶ）。
//
// - 表示している日のボード（session の board）から行を作り、読み込み直すたびに合わせる。まだ保存
//   していない欄（自動保存の待ち）と、まだ作っていない行は、画面の値を残す（エージェントの変更で
//   読み込み直しても、利用者の入力を失わないため。docs/design/01-architecture.md 4.1）。
// - 項目の追加・本文・完了と解消・解消メモ・分割は、自動保存（autosave.svelte.ts の ItemSaves）で送る。
//   削除・並べ替え・削除の取り消しは、その項目の保存を送り終えてから、1つずつ順に送る。
// - 取り消し（Mod+Z）とやり直し：項目の追加（とその直後の入力）・本文の入力・完了と解消・削除・
//   並べ替え・分割。持ち越し済みの項目と取りやめた TODO の状態・本文は戻さない（操作仕様書 2章）。
// - 入力中の欄（フォーカスがあり、最後の入力から EDITING_MS 以内か IME の変換中）を求める
//   （`report_ui_state` で知らせる中身。docs/design/01-architecture.md 4.1「項目の入力中」）。

import type {
  Board,
  BoardItem,
  CarriedTo,
  CloseReason,
  Deleted,
  EditingItem,
  Id,
  Item,
  ItemField,
  ItemKind,
  ItemStatus,
  LogicalDate,
  Millis,
  Origin,
  Position,
} from "./bindings";
import type { ItemSaves } from "./autosave.svelte";
import { core, errorOf, type CallError } from "./core";

/** 入力中とみなす、最後の入力からの時間（docs/design/01-architecture.md 4.1）。 */
export const EDITING_MS = 10_000;

/** 持ち越し回数をこの値以上で `--warn` にする（設定「操作」の持ち越し見直しの回数の既定値）。 */
export const CARRY_REVIEW_DEFAULT = 3;

/** 一覧の1行。 */
export interface Row {
  /** 画面での名前（読み込んだ項目は ID、画面で足した行は仮の名前）。行の間で変わらない。 */
  readonly key: string;
  /** 項目の ID。まだ作っていなければ `null`。 */
  id: Id | null;
  readonly kind: ItemKind;
  text: string;
  status: ItemStatus;
  closeReason: CloseReason | null;
  completedAt: Millis | null;
  resolutionNote: string | null;
  carryCount: number;
  /** 持ち越し先（持ち越し済みでなければ `null`）。 */
  carriedTo: CarriedTo | null;
  origin: Origin;
  /** 作ったエージェントの識別子。 */
  agentName: string | null;
  /** エージェントによる未確認の変更がある。 */
  unseen: boolean;
}

/** 本文や状態を変えられない理由（操作仕様書 2章）。 */
export type Locked = { reason: "carried"; to: CarriedTo } | { reason: "dropped" };

/** 画面に短く知らせること（文は画面が決める）。 */
export type BoardNotice =
  | { kind: "carried"; date: LogicalDate }
  | { kind: "dropped" }
  | { kind: "restore_source_changed" }
  | { kind: "restore_source_carried"; date: LogicalDate }
  | { kind: "rejected" };

/** 行の欄のうち、フォーカスを置くもの。 */
export type RowField = "text" | "memo";

/** 操作の後にフォーカスを置く場所（`offset` は本文の文字位置。`end` なら末尾）。 */
export interface FocusTarget {
  key: string;
  field: RowField;
  offset: number | "end";
}

export interface BoardDeps {
  items: ItemSaves;
  /** 表示している日を読み込み直す（送った内容を捨てたときなど）。 */
  reload(): void;
  notify(n: BoardNotice): void;
}

/** 取り消しの履歴の1件。`undo`・`redo` は何かを変えたら `true`（変えられなければ飛ばす）。 */
interface Entry {
  undo(): Promise<boolean>;
  redo(): Promise<boolean>;
}

/** 本文の入力の取り消し。続けて入力する間は1件にまとめる。 */
interface TextEntry extends Entry {
  key: string;
  /** 最後に入力した本文（取り消すとき、その後にエージェントが変えていないかを比べる）。 */
  after: string;
}

/** 取り消しの履歴の上限。 */
const HISTORY_LIMIT = 200;

let tmpSeq = 0;
const tmpKey = () => `new-${++tmpSeq}`;

const isClosed = (s: ItemStatus) => s === "done" || s === "resolved";

function rowOf(bi: BoardItem): Row {
  const i = bi.item;
  return {
    key: i.id,
    id: i.id,
    kind: i.kind,
    text: i.text,
    status: i.status,
    closeReason: i.closeReason,
    completedAt: i.completedAt,
    resolutionNote: i.resolutionNote,
    carryCount: i.carryCount,
    carriedTo: bi.carriedTo,
    origin: i.origin,
    agentName: bi.agentName,
    unseen: bi.unseen,
  };
}

/** 本文や状態を変えられない理由。変えられるなら `null`。 */
export function lockOf(row: Row): Locked | null {
  if (row.status === "dropped") return { reason: "dropped" };
  if (row.carriedTo !== null) return { reason: "carried", to: row.carriedTo };
  return null;
}

/** 解消メモを出すか：解消済みで、空でないメモがある（未解消の間は出さない。画面設計書 3章）。 */
export function showsMemo(row: Row): boolean {
  return row.kind === "blocker" && row.status === "resolved" && (row.resolutionNote ?? "").trim() !== "";
}

export class BoardModel {
  /** 表示している日。 */
  date = $state<LogicalDate | null>(null);
  todos = $state<Row[]>([]);
  blockers = $state<Row[]>([]);
  /** 解消メモを入力している行。 */
  memoOpen = $state<string | null>(null);
  /** 取り消し・やり直しできるか。 */
  canUndo = $state(false);
  canRedo = $state(false);

  readonly #deps: BoardDeps;
  /** 画面で作って、まだ読み込んだボードに出てきていない行（読み込みが作る前に始まっていたため）。 */
  readonly #unseenByServer = new Set<string>();
  /** この日に画面で削除した項目（削除の前に始まった読み込みで戻さないため）。 */
  readonly #deletedIds = new Set<Id>();
  #undo: Entry[] = [];
  #redo: Entry[] = [];
  #textEntry: TextEntry | null = null;
  /** 削除・並べ替えなどを1つずつ順に送る。 */
  #ops: Promise<unknown> = Promise.resolve();

  /** フォーカスのある欄（入力中の判定に使う）。 */
  focus = $state<{ key: string; field: RowField } | null>(null);
  #lastInputAt = 0;
  /** IME で変換している。 */
  composing = $state(false);

  constructor(deps: BoardDeps) {
    this.#deps = deps;
  }

  get items(): ItemSaves {
    return this.#deps.items;
  }

  list(kind: ItemKind): Row[] {
    return kind === "todo" ? this.todos : this.blockers;
  }

  row(key: string): Row | undefined {
    return this.todos.find((r) => r.key === key) ?? this.blockers.find((r) => r.key === key);
  }

  #rowById(id: Id): Row | undefined {
    return this.todos.find((r) => r.id === id) ?? this.blockers.find((r) => r.id === id);
  }

  // ---- 読み込み ----

  /**
   * 読み込んだボードを出す。別の日なら行を作り直し、同じ日なら、まだ保存していない欄とまだ作っていない
   * 行を残して合わせる（docs/design/01-architecture.md 4.1）。
   */
  load(board: Board) {
    if (board.date !== this.date) {
      this.date = board.date;
      this.todos = board.todos.map(rowOf);
      this.blockers = board.blockers.map(rowOf);
      this.memoOpen = null;
      this.focus = null;
      this.#unseenByServer.clear();
      this.#deletedIds.clear();
      this.#undo = [];
      this.#redo = [];
      this.#textEntry = null;
      this.#syncHistory();
      return;
    }
    this.todos = this.#merge(this.todos, board.todos);
    this.blockers = this.#merge(this.blockers, board.blockers);
  }

  #merge(rows: Row[], server: BoardItem[]): Row[] {
    const byId = new Map(rows.filter((r) => r.id !== null).map((r) => [r.id!, r]));
    const ids = new Set(server.map((bi) => bi.item.id));
    const merged: Row[] = [];
    for (const bi of server) {
      if (this.#deletedIds.has(bi.item.id)) continue;
      const old = byId.get(bi.item.id);
      const next = rowOf(bi);
      if (old === undefined) {
        merged.push(next);
        continue;
      }
      this.#unseenByServer.delete(old.key);
      const unsaved = this.items.unsaved(old.key);
      // 行の名前は変えない（画面の部品とフォーカスを保つ）
      const row: Row = { ...next, key: old.key };
      if (unsaved.has("text") || old.text.trim() === "") row.text = old.text;
      if (unsaved.has("resolutionNote")) row.resolutionNote = old.resolutionNote;
      if (unsaved.has("status")) {
        row.status = old.status;
        row.completedAt = old.completedAt;
      }
      merged.push(row);
    }
    // 画面で足した、読み込んだボードにまだ無い行は、前にあった行の後ろに残す
    let prev: string | null = null;
    for (const r of rows) {
      const local = r.id === null || (this.#unseenByServer.has(r.key) && !ids.has(r.id));
      if (local) merged.splice(prev === null ? 0 : merged.findIndex((m) => m.key === prev) + 1, 0, r);
      if (merged.some((m) => m.key === r.key)) prev = r.key;
    }
    return merged;
  }

  /** 保存して作った（ID が決まった）。`ItemSaves` の `saved` から呼ぶ。 */
  saved(key: string, item: Item) {
    const row = this.row(key) ?? this.#rowById(item.id);
    if (row !== undefined && row.id === null) {
      row.id = item.id;
      this.#unseenByServer.add(row.key);
    }
  }

  /** 再試行しない失敗で、送った内容を捨てた（`ItemSaves` の `rejected` から呼ぶ）。 */
  rejected(key: string, e: CallError) {
    // 作れなかった行は、もう作らないので画面からも消す（作れていれば、読み込み直すと出てくる）
    const row = this.row(key);
    if (row !== undefined && row.id === null) {
      const list = this.list(row.kind);
      list.splice(list.indexOf(row), 1);
    }
    this.#deps.notify(this.#noticeOf(e));
    this.#deps.reload();
  }

  #noticeOf(e: CallError): BoardNotice {
    if (e.kind === "conflict" && e.reason === "carried_over") return { kind: "carried", date: e.carriedToDate };
    if (e.kind === "invalid_argument" && e.reason === "dropped") return { kind: "dropped" };
    return { kind: "rejected" };
  }

  // ---- 入力中の状態 ----

  /** 欄にフォーカスが入った・外れた（`null`）。 */
  setFocus(focus: { key: string; field: RowField } | null) {
    if (focus?.key !== this.focus?.key) this.#textEntry = null;
    this.focus = focus;
  }

  /**
   * 入力中の項目の欄（docs/design/01-architecture.md 4.1「項目の入力中」）：フォーカスがあり（アプリが
   * 前面にある）、最後の入力から EDITING_MS 以内か IME の変換中の欄と、まだ保存し終えていない欄。
   */
  editingItems(now: number, active: boolean): EditingItem[] {
    const out = this.items.unsavedFields();
    const f = this.focus;
    if (f !== null && active && (this.composing || now - this.#lastInputAt < EDITING_MS)) {
      const id = this.row(f.key)?.id ?? null;
      const field: ItemField = f.field === "text" ? "text" : "resolution_note";
      if (id !== null && !out.some((e) => e.itemId === id && e.field === field)) out.push({ itemId: id, field });
    }
    return out;
  }

  /** 入力中でなくなる時刻（それまでに知らせ直す）。入力中でなければ `null`。 */
  editingUntil(): number | null {
    return this.focus === null || this.composing ? null : this.#lastInputAt + EDITING_MS;
  }

  // ---- 位置 ----

  /** 行 `key` を、いまの並びのとおりに入れる位置（前の作った行の後ろ、無ければ次の作った行の前）。 */
  positionOf(key: string): Position {
    const row = this.row(key);
    if (row === undefined) return { at: "last" };
    const list = this.list(row.kind);
    const i = list.indexOf(row);
    for (let j = i - 1; j >= 0; j--) if (list[j].id !== null) return { at: "after", id: list[j].id! };
    for (let j = i + 1; j < list.length; j++) if (list[j].id !== null) return { at: "before", id: list[j].id! };
    return { at: "first" };
  }

  // ---- 操作 ----

  /** 変えられない行への操作。知らせて `true`。 */
  #refuse(row: Row): boolean {
    const lock = lockOf(row);
    if (lock === null) return false;
    this.#deps.notify(lock.reason === "carried" ? { kind: "carried", date: lock.to.date } : { kind: "dropped" });
    return true;
  }

  /** 本文や状態を変えられない行なら知らせて `true`（文字の入力など、画面が先に確かめる操作）。 */
  refuseEdit(key: string): boolean {
    const row = this.row(key);
    return row === undefined || this.#refuse(row);
  }

  #push(entry: Entry) {
    this.#undo.push(entry);
    if (this.#undo.length > HISTORY_LIMIT) this.#undo.shift();
    this.#redo = [];
    this.#textEntry = null;
    this.#syncHistory();
  }

  #syncHistory() {
    this.canUndo = this.#undo.length > 0;
    this.canRedo = this.#redo.length > 0;
  }

  /** 1つずつ順に行う（削除・並べ替えなど）。失敗したら読み込み直す。 */
  #queue<T>(op: () => Promise<T>): Promise<T | null> {
    const run = this.#ops.then(op).catch((e) => {
      this.#failed(errorOf(e));
      return null;
    });
    this.#ops = run;
    return run;
  }

  #failed(e: CallError) {
    // 保存に失敗している間（再試行の待ち）は、操作を行わなかっただけ。表示は保存状態が伝える
    if (e.kind === "storage" || e.kind === "not_ready") return;
    if (e.kind === "restore_refused") {
      this.#deps.notify(
        e.reason === "source_changed"
          ? { kind: "restore_source_changed" }
          : { kind: "restore_source_carried", date: e.carriedToDate },
      );
    } else {
      this.#deps.notify(this.#noticeOf(e));
    }
    this.#deps.reload();
  }

  #newRow(kind: ItemKind, text: string): Row {
    return {
      key: tmpKey(),
      id: null,
      kind,
      text,
      status: "open",
      closeReason: null,
      completedAt: null,
      resolutionNote: null,
      carryCount: 0,
      carriedTo: null,
      origin: "human",
      agentName: null,
      unseen: false,
    };
  }

  /**
   * 空の行を足す（Enter）。`ref` の前か後、`ref` が無ければ末尾。本文が入るまでは作らない
   * （何も書かずに離れた行は消える）。足した行と、その直後の入力は、1回の取り消しで戻る。
   */
  add(kind: ItemKind, ref: string | null = null, where: "before" | "after" = "after"): FocusTarget | null {
    if (this.date === null) return null;
    const list = this.list(kind);
    const row = this.#newRow(kind, "");
    const i = ref === null ? -1 : list.findIndex((r) => r.key === ref);
    list.splice(i < 0 ? list.length : where === "before" ? i : i + 1, 0, row);
    this.items.create(row.key, { date: this.date, kind, position: () => this.positionOf(row.key) }, "");
    const entry = this.#rowEntry(row.key);
    this.#push(entry);
    // 足した直後の入力は、足したことと1つにまとめる
    this.#textEntry = { ...entry, key: row.key, after: "" };
    return { key: row.key, field: "text", offset: 0 };
  }

  /** 行を足した・分けた操作の取り消し：その行を消す（作っていれば削除し、やり直しで戻す）。 */
  #rowEntry(key: string): Entry {
    let deleted: Deleted | null = null;
    return {
      undo: async () => {
        const row = this.row(key);
        if (row === undefined) return false;
        deleted = await this.#removeRow(row);
        return true;
      },
      redo: async () => {
        if (deleted === null) return false;
        return this.#restore(deleted);
      },
    };
  }

  /** 本文を入力した。 */
  editText(key: string, text: string) {
    const row = this.row(key);
    if (row === undefined || lockOf(row) !== null) return;
    const before = row.text;
    row.text = text;
    this.#lastInputAt = Date.now();
    // 空白だけの本文は保存しない（離れたら行を消す。操作仕様書 2章）
    if (text.trim() !== "") this.items.editText(key, text);
    if (this.#textEntry?.key === key) {
      this.#textEntry.after = text;
    } else {
      const entry: TextEntry = {
        key,
        after: text,
        undo: async () => this.#setText(key, entry.after, before),
        redo: async () => this.#setText(key, before, entry.after),
      };
      this.#push(entry);
      this.#textEntry = entry;
    }
  }

  /** 取り消し・やり直しで本文を `to` にする。その後にエージェントが変えていたら（`from` でない）変えない。 */
  #setText(key: string, from: string, to: string): boolean {
    const row = this.row(key);
    if (row === undefined || lockOf(row) !== null || row.text !== from || to.trim() === "") return false;
    row.text = to;
    this.items.editText(key, to);
    return true;
  }

  /** IME の変換を始めた・終えた。 */
  setComposing(on: boolean) {
    this.composing = on;
    this.#lastInputAt = Date.now();
  }

  /** 解消メモを入力した（空なら消す）。 */
  editMemo(key: string, text: string) {
    const row = this.row(key);
    if (row === undefined || this.#refuse(row)) return;
    row.resolutionNote = text;
    this.#lastInputAt = Date.now();
    this.items.editResolutionNote(key, text);
  }

  /** 解消メモの欄を開く（解消済みの障害だけ）。開いたら `true`。 */
  openMemo(key: string): boolean {
    const row = this.row(key);
    if (row === undefined || row.status !== "resolved" || this.#refuse(row)) return false;
    this.memoOpen = key;
    return true;
  }

  /** 解消メモの欄を閉じる（Enter・Esc・離れたとき）。まだ保存していない入力はすぐ送る。 */
  closeMemo() {
    const key = this.memoOpen;
    this.memoOpen = null;
    if (key !== null) void this.items.flush(key);
  }

  /**
   * 完了・解消を切り替える（Mod+Enter・チェック欄）。障害を解消したら、解消メモの欄を開く
   * （フォーカスの行き先を返す）。未解消に戻したら、まだ保存していないメモを送ってから欄を閉じる。
   */
  toggle(key: string): FocusTarget | null {
    const row = this.row(key);
    if (row === undefined || this.#refuse(row)) return null;
    const before = { status: row.status, completedAt: row.completedAt };
    const closing = !isClosed(row.status);
    const closedAt = closing ? Date.now() : null;
    const after = this.#setStatus(row, closing, closedAt);
    this.#push({
      undo: async () => this.#revertStatus(key, after, before),
      redo: async () => this.#revertStatus(key, before.status, { status: after, completedAt: closedAt }),
    });
    if (row.kind === "blocker" && closing) {
      this.memoOpen = key;
      return { key, field: "memo", offset: "end" };
    }
    if (this.memoOpen === key) this.closeMemo();
    return null;
  }

  #setStatus(row: Row, closing: boolean, closedAt: Millis | null): ItemStatus {
    const shown = row.status;
    row.status = closing ? (row.kind === "todo" ? "done" : "resolved") : "open";
    row.completedAt = closedAt;
    this.items.setClosed(row.key, shown, closedAt);
    return row.status;
  }

  /**
   * 取り消し・やり直しで状態を `to` に戻す（完了時刻も元に戻す。解消メモの欄は開かない）。その後に
   * エージェントが状態を変えていた（`from` でない）ら戻さない（操作仕様書 2章）。
   */
  #revertStatus(key: string, from: ItemStatus, to: { status: ItemStatus; completedAt: Millis | null }): boolean {
    const row = this.row(key);
    if (row === undefined || lockOf(row) !== null || row.status !== from) return false;
    this.#setStatus(row, isClosed(to.status), isClosed(to.status) ? to.completedAt : null);
    if (this.memoOpen === key && row.status !== "resolved") this.closeMemo();
    return true;
  }

  /**
   * 行の途中の Enter：カーソルの位置 `offset` で2件に分ける（データモデル設計書 3章「行の分割」）。
   * 行頭では直上に、行末では直下に空の行を足す（どちらかが空白だけになるときは分けない）。
   */
  splitAt(key: string, offset: number): FocusTarget | null {
    const row = this.row(key);
    if (row === undefined || this.date === null) return null;
    const first = row.text.slice(0, offset);
    const second = row.text.slice(offset);
    if (first.trim() === "" && row.text.trim() !== "") return this.add(row.kind, key, "before");
    if (second.trim() === "") return this.add(row.kind, key, "after");
    // 本文を変えるので、持ち越し済みの項目と取りやめた TODO は分けない
    if (this.#refuse(row)) return null;
    const before = row.text;
    const list = this.list(row.kind);
    const created = this.#newRow(row.kind, second);
    row.text = first;
    list.splice(list.indexOf(row) + 1, 0, created);
    this.#lastInputAt = Date.now();
    this.items.split(key, created.key, { date: this.date, kind: row.kind }, first, second);
    const removeCreated = this.#rowEntry(created.key);
    this.#push({
      undo: async () => {
        const r = this.row(key);
        if (r === undefined) return false;
        const createdRow = this.row(created.key);
        // 分けた後の2行の本文をつなぎ直す（分けた後に入力していれば、その本文で）
        const joined = createdRow === undefined ? before : r.text + createdRow.text;
        await removeCreated.undo();
        if (lockOf(r) === null && joined.trim() !== "") {
          r.text = joined;
          this.items.editText(key, joined);
        }
        return true;
      },
      redo: async () => {
        const r = this.row(key);
        if (r === undefined || lockOf(r) !== null) return false;
        return this.splitAt(key, first.length) !== null;
      },
    });
    return { key: created.key, field: "text", offset: 0 };
  }

  /**
   * 離れた行の後始末：まだ保存していない入力はすぐ送り、何も書かずに離れた行（本文が空白だけ）は消す
   * （操作仕様書 2章）。
   */
  leave(key: string) {
    const row = this.row(key);
    if (row === undefined) return;
    if (this.memoOpen === key) this.closeMemo();
    if (row.text.trim() !== "") {
      void this.items.flush(key);
    } else if (row.id === null) {
      void this.#removeRow(row);
    } else {
      // 本文を消して離れた項目は、削除として取り消せるようにする
      this.remove(key);
    }
  }

  /** 行を消す（Mod+Backspace・空の行の Backspace）。取り消せる。 */
  remove(key: string) {
    const row = this.row(key);
    if (row === undefined) return;
    let deleted: Deleted | null = null;
    let index = this.list(row.kind).indexOf(row);
    const kind = row.kind;
    const snapshot = { ...row };
    this.#push({
      undo: async () => {
        if (deleted === null) {
          // 作る前に消した行は、画面にだけ戻す
          if (snapshot.id !== null || snapshot.text.trim() === "") return false;
          const back = this.#newRow(kind, snapshot.text);
          this.list(kind).splice(Math.min(index, this.list(kind).length), 0, back);
          if (this.date !== null) {
            this.items.create(back.key, { date: this.date, kind, position: () => this.positionOf(back.key) }, back.text);
          }
          return true;
        }
        return this.#restore(deleted);
      },
      redo: async () => {
        const r = deleted === null ? undefined : this.#rowById(deleted.id);
        if (r === undefined) return false;
        index = this.list(r.kind).indexOf(r);
        deleted = await this.#removeRow(r);
        return true;
      },
    });
    void this.#removeRow(row).then((d) => (deleted = d));
  }

  /**
   * 行を画面から消し、作っていれば削除する（作る途中なら、作り終えるのを待って）。削除の結果
   * （取り消しに使う）を返す。作る前の行なら `null`。
   */
  async #removeRow(row: Row): Promise<Deleted | null> {
    const list = this.list(row.kind);
    const index = list.indexOf(row);
    if (index >= 0) list.splice(index, 1);
    if (this.memoOpen === row.key) this.memoOpen = null;
    if (row.id === null && this.items.discard(row.key)) return null;
    const deleted = await this.#queue(() => this.items.run(row.key, (id) => core.deleteItem(id)));
    if (deleted === null) {
      // 削除できなかった（保存に失敗している間など）。行を戻す（まだ保存していない入力を残すため）
      const l = this.list(row.kind);
      if (!l.includes(row)) l.splice(Math.min(index, l.length), 0, row);
      return null;
    }
    this.#deletedIds.add(deleted.id);
    return deleted;
  }

  /** 削除を取り消す。戻せなければ（持ち越し元が変わったなど）理由を知らせて `false`。 */
  async #restore(deleted: Deleted): Promise<boolean> {
    const item = await this.#queue(async () => {
      const it = await core.restoreItem(deleted);
      this.#deletedIds.delete(deleted.id);
      return it;
    });
    if (item === null) return false;
    this.#deps.reload();
    return true;
  }

  /** 項目を同じ種類の中で1つ上（-1）・下（+1）へ動かす（Alt+↑ / Alt+↓）。 */
  move(key: string, dir: -1 | 1) {
    const row = this.row(key);
    if (row === undefined) return;
    this.moveTo(key, this.list(row.kind).indexOf(row) + dir);
  }

  /** 項目を同じ種類の中の `j` 番目（0 始まり。動かした後の位置）へ動かす（ドラッグ）。 */
  moveTo(key: string, j: number) {
    const row = this.row(key);
    if (row === undefined) return;
    const list = this.list(row.kind);
    const i = list.indexOf(row);
    if (j === i || j < 0 || j >= list.length) return;
    const from = list[i - 1]?.key ?? null;
    this.#moveTo(row, j);
    const to = list[j - 1]?.key ?? null;
    this.#push({
      undo: async () => this.#moveAfter(key, from),
      redo: async () => this.#moveAfter(key, to),
    });
  }

  /** 取り消し・やり直しで、行 `key` を行 `after` の後ろ（`null` なら先頭）へ動かす。 */
  #moveAfter(key: string, after: string | null): boolean {
    const row = this.row(key);
    if (row === undefined) return false;
    const list = this.list(row.kind);
    const without = list.filter((r) => r !== row);
    const j = after === null ? 0 : without.findIndex((r) => r.key === after) + 1;
    if (after !== null && j === 0) return false;
    this.#moveTo(row, j);
    return true;
  }

  #moveTo(row: Row, j: number) {
    const list = this.list(row.kind);
    list.splice(list.indexOf(row), 1);
    list.splice(j, 0, row);
    // 本文がまだ無い（作っていない）行は、作るときにいまの位置に入る
    if (row.id === null && row.text.trim() === "") return;
    void this.#queue(() => this.items.run(row.key, (id) => core.moveItem(id, this.positionOf(row.key))));
  }

  /** 取り消す（Mod+Z）。変えられない記録（行が無い・エージェントが変えた）は飛ばす。 */
  async undo() {
    this.#textEntry = null;
    for (let e = this.#undo.pop(); e !== undefined; e = this.#undo.pop()) {
      this.#redo.push(e);
      this.#syncHistory();
      if (await e.undo()) return;
    }
    this.#syncHistory();
  }

  /** やり直す（Mod+Shift+Z、Windows・Linux は Ctrl+Y も）。 */
  async redo() {
    this.#textEntry = null;
    for (let e = this.#redo.pop(); e !== undefined; e = this.#redo.pop()) {
      this.#undo.push(e);
      this.#syncHistory();
      if (await e.redo()) return;
    }
    this.#syncHistory();
  }

  /** 削除・並べ替えなどを送り終えるまで待つ（テスト用）。 */
  async settled() {
    await this.#ops;
  }
}

