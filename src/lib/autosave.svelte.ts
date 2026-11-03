// 自動保存（要件 F-08、docs/design/01-architecture.md 4.1「自動保存」、画面設計書 3章「保存状態」）。
// 見た目は持たない（整理モードの一覧 src/lib/board.svelte.ts と、振り返りのエディタ
// src/board/ReflectionEditor.svelte がこれを呼ぶ）。
//
// - 入力のたびに知らせると（`edited`・`edit`）、まだ保存していない最初の入力から SAVE_DELAY_MS 後に
//   保存する。入力が続いても待ち直さない（入力中に強制終了しても、失うのは直前の短い間の入力だけに
//   するため。テスト計画書 4.1）。フォーカスが外れたときなどは `flush` ですぐ保存する。
// - 同じ対象（ノート・振り返りは日付ごと、項目は1件ごと）への保存は一度に1つだけ送り、前の保存の
//   応答を待ってから次を送る。項目は、待つ間に変えた欄を次の保存にまとめる。
// - 保存先の問題で失敗したら（retryable）、内容を持ったまま、間を空けて自動で再試行する。
//   その間は「保存できていません」（SaveStatus.failing）。入力は続けられる。
// - 行の分割は、2行の本文を持ったまま再試行し、送る時点の本文で分ける（ItemSaves.split）。
// - ノート・振り返りの保存の行き違い（rev が古い）では、最新の内容をエディタに取り込んでから
//   （text_merge.ts）、保存し直す。項目は版の番号を持たず、変えた欄だけを送る。

import type {
  EditingItem,
  Id,
  Item,
  ItemFields,
  ItemKind,
  ItemStatus,
  LogicalDate,
  Millis,
  Position,
  Snapshot,
  StatusChange,
  TextDoc,
} from "./bindings";
import { core, CoreError, errorOf, type CallError } from "./core";
import { mergeText, unsavedLines } from "./text_merge";

/** まだ保存していない最初の入力から、保存を送るまでの時間（docs/design/01-architecture.md 4.1）。 */
export const SAVE_DELAY_MS = 250;

/** 保存に失敗してから再試行するまでの時間（続けて失敗するたびに次の値。最後の値を繰り返す）。 */
export const RETRY_DELAYS_MS = [1000, 2000, 4000, 8000, 10000] as const;

/**
 * 内容を持ったまま再試行する失敗か。保存先の問題（`storage`）、データベースをまだ開いていない・
 * 閉じた（`not_ready`）、コマンドの処理が途中で止まった（`internal`）。ほかの失敗（引数の誤り・
 * 項目が無い・持ち越し済みなど）は、送り直しても成功しないので再試行しない。
 */
export function isRetryable(e: CallError): boolean {
  return e.kind === "storage" || e.kind === "not_ready" || e.kind === "internal";
}

/** 保存の状態（画面設計書 3章「上部のバー」の保存状態）。 */
export class SaveStatus {
  /** まだ保存していない入力がある（保存を待っている・送っている）。保存中の点を出す。 */
  saving = $state(false);
  /** 保存に失敗して再試行している（「保存できていません」）。成功するまで `true`。 */
  failing = $state(false);
  /** 再試行している失敗の原因（原因の文を `kind` から決める）。失敗していなければ `null`。 */
  error = $state<CallError | null>(null);

  readonly #savers = new Set<Saver<unknown>>();

  /** @internal */
  add(saver: Saver<unknown>) {
    this.#savers.add(saver);
  }

  /** @internal */
  remove(saver: Saver<unknown>) {
    this.#savers.delete(saver);
    this.update();
  }

  /** @internal */
  update() {
    let saving = false;
    let error: CallError | null = null;
    for (const s of this.#savers) {
      saving ||= s.busy;
      error ??= s.failure;
    }
    this.saving = saving;
    this.failing = error !== null;
    this.error = error;
  }

  /** 失敗しているものを、待たずにすぐ再試行する（「再試行」の操作）。 */
  retryNow() {
    for (const s of this.#savers) if (s.failure !== null) void s.flush();
  }

  /** まだ保存していないものを、すべてすぐ保存する（アプリが前面から外れたときなど）。 */
  async flushAll() {
    await Promise.all([...this.#savers].map((s) => s.flush()));
  }
}

/** 送った保存の結果：済んだか、すぐ送り直すか（ノートの行き違いを取り込んだ後など）。 */
type Sent = "done" | "again";

/**
 * 1つの対象の保存の待ち行列：入力から SAVE_DELAY_MS 後に送り、一度に1つだけ送り、失敗したら
 * 間を空けて再試行する。
 */
abstract class Saver<P> {
  readonly #status: SaveStatus;
  /** まだ保存していない最初の入力の時刻。無ければ `null`。 */
  #pendingSince: number | null = null;
  #immediate = false;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #inFlight: Promise<void> | null = null;
  /** 続けて失敗した回数。 */
  #failures = 0;
  #failure: CallError | null = null;
  #closed = false;
  /** 閉じるのを、保存し終えるまで待っている（`close`）。 */
  #closing = false;

  constructor(status: SaveStatus) {
    this.#status = status;
    status.add(this);
  }

  /** 送る内容を取り出す。送るものが無ければ `null`。 */
  protected abstract take(): P | null;
  /** 送る。失敗したら投げる。 */
  protected abstract send(p: P): Promise<Sent>;
  /** 再試行する失敗で、取り出した内容を戻す（待つ間に変えた内容と合わせる）。 */
  protected abstract putBack(p: P): void;
  /** 再試行しない失敗。内容は捨てる。 */
  protected abstract reject(p: P, e: CallError): void;
  /** 内容を持ったまま再試行する失敗か。 */
  protected retryable(e: CallError): boolean {
    return isRetryable(e);
  }
  /** 送り終えた後（成功・失敗とも）。 */
  protected settled(): void {}

  /** まだ保存していない入力がある（保存を待っている・送っている・再試行を待っている）。 */
  get busy(): boolean {
    return this.#pendingSince !== null || this.#inFlight !== null;
  }

  /** 送っている途中か。 */
  protected get sending(): boolean {
    return this.#inFlight !== null;
  }

  /** 再試行している失敗。無ければ `null`。 */
  get failure(): CallError | null {
    return this.#failure;
  }

  /** 保存していない入力ができた。 */
  protected markPending(immediate = false) {
    if (this.#closed) return;
    this.#pendingSince ??= Date.now();
    this.#immediate ||= immediate;
    this.#schedule();
    this.#status.update();
  }

  #schedule() {
    if (this.#closed || this.#timer !== null || this.#inFlight !== null) return;
    if (this.#pendingSince === null) return;
    const delay =
      this.#failure !== null
        ? RETRY_DELAYS_MS[Math.min(this.#failures, RETRY_DELAYS_MS.length) - 1]
        : this.#immediate
          ? 0
          : Math.max(0, this.#pendingSince + SAVE_DELAY_MS - Date.now());
    this.#timer = setTimeout(() => {
      this.#timer = null;
      void this.#run();
    }, delay);
  }

  #clearTimer() {
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
  }

  /** 今の内容を1回送り、終わるのを待つ。 */
  async #run() {
    if (this.#inFlight !== null) return;
    const p = this.take();
    const since = this.#pendingSince ?? Date.now();
    this.#pendingSince = null;
    this.#immediate = false;
    if (p === null) {
      // 送るものが無くなった（保存済みの本文に戻したなど）。再試行するものも無いので、失敗中を終える
      this.#failures = 0;
      this.#failure = null;
      this.#status.update();
      return;
    }
    this.#inFlight = (async () => {
      try {
        const sent = await this.send(p);
        this.#failures = 0;
        this.#failure = null;
        if (sent === "again") this.markPending(true);
      } catch (e) {
        const error = errorOf(e);
        if (this.retryable(error)) {
          this.putBack(p);
          this.#failures++;
          this.#failure = error;
          this.#pendingSince ??= since;
        } else {
          // 再試行しない失敗の後は、再試行の待ちと「保存できていません」を続けない（内容は捨てた）
          this.#failures = 0;
          this.#failure = null;
          this.reject(p, error);
        }
      }
    })();
    this.#status.update();
    await this.#inFlight;
    this.#inFlight = null;
    this.settled();
    this.#schedule();
    if (this.#closing && !this.busy) this.#finish();
    this.#status.update();
  }

  /**
   * 待たずにすぐ保存する（フォーカスが外れたとき、アプリが前面から外れたとき）。保存し終えたら
   * `true`、失敗したか（再試行は後ろで続く）、送り直しが上限の回数を超えて（行き違いが続いた）まだ
   * 保存していないものがあれば `false` で解決する。
   */
  async flush(): Promise<boolean> {
    // 行き違いの取り込みのたびに送り直すので、何度か回る。無限に回らないよう上限を置く
    for (let round = 0; round < 10 && !this.#closed; round++) {
      this.#clearTimer();
      if (this.#inFlight !== null) {
        await this.#inFlight;
        continue;
      }
      if (this.#pendingSince === null) return true;
      await this.#run();
      if (this.#failure !== null) return false;
    }
    if (this.busy) this.#schedule();
    return !this.busy;
  }

  /**
   * 使うのをやめる（日付を移ったときなど）。まだ保存していないものは先に送る。保存に失敗して
   * いれば、保存し終えるまで再試行を続けてから片付ける（入力を捨てないため）。保存し終えて片付けたら
   * `true`、再試行を続けているなら `false` で解決する。
   */
  async close(): Promise<boolean> {
    this.#closing = true;
    await this.flush();
    if (this.busy) return false;
    this.#finish();
    return true;
  }

  #finish() {
    if (this.#closed) return;
    this.#closed = true;
    this.#clearTimer();
    this.#status.remove(this);
  }
}

// ---- ノート・振り返り ----

/** エディタの中身（エディタの部品が渡す）。 */
export interface DocEditor {
  /** いまの本文。 */
  text(): string;
  /** 本文を置き換える（取り込みの結果）。カーソルと選択範囲を保つのはエディタの役目。 */
  replace(text: string): void;
}

/**
 * その日のノートか振り返りの自動保存（docs/design/01-architecture.md 4.1「保存の行き違いを防ぐ」）。
 * もとにした版（`saved`）を持ち、保存に添える。版が古いと返ったら、最新を取り込んでから保存し直す。
 */
export class DocSaver extends Saver<Snapshot> {
  readonly doc: TextDoc;
  readonly date: LogicalDate;
  #editor: DocEditor;
  #saved: Snapshot;
  /** 送っている間に届いた、取り込む最新（送り終えてから取り込む）。 */
  #deferred: Snapshot | null = null;

  /** `saved` は読み込んだ本文と版（エディタはその本文で始める）。 */
  constructor(status: SaveStatus, doc: TextDoc, date: LogicalDate, saved: Snapshot, editor: DocEditor) {
    super(status);
    this.doc = doc;
    this.date = date;
    this.#saved = saved;
    this.#editor = editor;
  }

  /**
   * 使うのをやめる（`Saver.close`）。この後は渡されたエディタに触れない（エディタは別の日の本文を
   * 映しうるため）。閉じる時点の本文を手元に写し、保存に失敗していれば、それを保存し終えるまで
   * 再試行する。
   */
  close(): Promise<boolean> {
    let text = this.#editor.text();
    this.#editor = {
      text: () => text,
      replace: (t: string) => {
        text = t;
      },
    };
    return super.close();
  }

  /** 最後に保存した（取り込んだ）本文と版。 */
  get saved(): Snapshot {
    return this.#saved;
  }

  /** エディタで入力した。 */
  edited() {
    if (this.#editor.text() !== this.#saved.body) this.markPending();
  }

  /**
   * コアの最新（エージェントの変更の後に読み直したもの）を、まだ保存していない入力を残したまま
   * エディタに取り込み、もとを進める。もとより新しくなければ何もしない。保存を送っている間は、
   * 送り終えてから取り込む（送った内容がもとに入る前に取り込むと、利用者の行が二重になるため）。
   */
  absorb(latest: Snapshot) {
    if (this.sending) {
      if (latest.rev > (this.#deferred?.rev ?? this.#saved.rev)) this.#deferred = latest;
      return;
    }
    if (this.#merge(latest)) this.markPending();
  }

  /**
   * 最新 `latest` をエディタに取り込み、もとを進める。もとより新しくなければ何もしない。
   * 取り込んだ後に、まだ保存していない入力が残っていれば `true`。
   */
  #merge(latest: Snapshot): boolean {
    if (latest.rev <= this.#saved.rev) return false;
    const editor = this.#editor.text();
    const merged = mergeText(this.#saved.body, latest.body, editor);
    if (merged !== editor) this.#editor.replace(merged);
    this.#saved = latest;
    return merged !== latest.body;
  }

  /**
   * 守る行のうち「未保存の変更がある行」（もとの本文の行番号。`saved.rev` を添えて知らせる。
   * docs/design/01-architecture.md 4.1）。
   */
  unsavedLines(): number[] {
    // カーソルを動かすたびに知らせ直すので、同じ本文どうしなら前の結果を使う（差分は重いため）
    const base = this.#saved.body;
    const editor = this.#editor.text();
    const c = this.#unsavedCache;
    if (c === null || c.base !== base || c.editor !== editor) {
      this.#unsavedCache = { base, editor, lines: unsavedLines(base, editor) };
    }
    return [...this.#unsavedCache!.lines];
  }

  #unsavedCache: { base: string; editor: string; lines: number[] } | null = null;

  protected take(): Snapshot | null {
    const body = this.#editor.text();
    return body === this.#saved.body ? null : { body, rev: this.#saved.rev };
  }

  protected async send(p: Snapshot): Promise<Sent> {
    const r = await core.saveTextDoc(this.doc, this.date, p.body, p.rev);
    if (r.result === "saved") {
      if (r.rev >= this.#saved.rev) this.#saved = { body: p.body, rev: r.rev };
      return "done";
    }
    // 行き違い：取り込んでから、すぐ保存し直す。送っている間に届いた、これより新しい最新は、
    // 送り終えてから取り込む（settled）
    this.#merge({ body: r.body, rev: r.rev });
    return this.#editor.text() === this.#saved.body ? "done" : "again";
  }

  protected settled() {
    const deferred = this.#deferred;
    this.#deferred = null;
    if (deferred !== null) this.absorb(deferred);
  }

  protected putBack() {
    // 本文はエディタにあるので、戻すものは無い
  }

  /**
   * 本文の保存の失敗は、どれも内容を持ったまま再試行する（送り直して成功しない失敗は、本文の保存には
   * 無いはずで、あれば画面の不具合。捨てると、エディタに残った本文が保存されないまま「保存済み」に
   * 見えるため、「保存できていません」を出し続ける）。
   */
  protected retryable(): boolean {
    return true;
  }

  protected reject() {
    // retryable がいつも true なので呼ばれない
  }
}

// ---- 項目 ----

/** 項目の保存で送る欄（同じ欄は最後の値だけ。状態は最初の「変える前の値」と操作の時刻を残す）。 */
export type ItemPatch = ItemFields;

function mergePatch(older: ItemPatch, newer: ItemPatch): ItemPatch {
  const status: StatusChange | undefined =
    older.status && newer.status
      ? { ...newer.status, before: older.status.before, changedAt: older.status.changedAt }
      : (newer.status ?? older.status);
  return {
    ...(newer.text ?? older.text) !== undefined && { text: newer.text ?? older.text },
    ...(newer.resolutionNote ?? older.resolutionNote) !== undefined && {
      resolutionNote: newer.resolutionNote ?? older.resolutionNote,
    },
    ...(status !== undefined && { status }),
  };
}

const isEmptyPatch = (p: ItemPatch) =>
  p.text === undefined && p.resolutionNote === undefined && p.status === undefined;

/** まだ作っていない項目を作るための値（`addItem` の本文以外）。 */
export interface NewItem {
  date: LogicalDate;
  kind: ItemKind;
  /**
   * 入れる位置。関数なら送る時点で決める（前後の行がまだ作られていないうちに入力した行を、作る時点の
   * 並びの位置に入れるため）。
   */
  position: Position | (() => Position);
}

/**
 * 行の分割で作る項目（docs/design/01-architecture.md 4.1「自動保存」の行の分割）。元の項目 `from`
 * （`ItemSaves` の名前）の本文を `first` に、作る項目の本文を、この項目の本文にする。
 */
interface SplitItem {
  from: string;
  date: LogicalDate;
  kind: ItemKind;
  /** 元の項目の本文（送る時点の値。分割を送り終えるまでの元の行の入力は、ここにまとめる）。 */
  first: string;
}

/** 利用者が入力する欄の名前（`ItemSaves.unsaved`）。 */
export type ItemFieldName = "text" | "resolutionNote" | "status";

/** 項目の保存の結果の知らせ先。 */
export interface ItemSaveEvents {
  /** 保存した（作った）。`key` は `edit` などに渡したもの。 */
  saved?(key: string, item: Item): void;
  /** 再試行しない失敗で、送った内容を捨てた（画面は表示を読み込み直す）。 */
  rejected?(key: string, error: CallError, patch: ItemPatch): void;
}

class ItemSaver extends Saver<ItemPatch> {
  readonly key: string;
  id: Id | null;
  #create: NewItem | null;
  #split: SplitItem | null;
  /** 分割を送っている間に入力した、元の行の本文（送り終えたら元の項目の保存に回す）。 */
  #lateFirst: string | undefined;
  #pending: ItemPatch = {};
  /** 送っている欄（入力中とみなす欄を知らせるため）。 */
  #sendingPatch: ItemPatch | null = null;
  readonly #events: ItemSaveEvents;
  /** 名前から、ほかの項目の待ち行列を引く（分割の元の項目。無ければ作る）。 */
  readonly #lookup: (key: string) => ItemSaver | undefined;
  /** 作り終えた（ID が決まった）とき。 */
  onCreated?: (s: ItemSaver) => void;

  constructor(
    status: SaveStatus,
    key: string,
    id: Id | null,
    create: NewItem | null,
    events: ItemSaveEvents,
    lookup: (key: string) => ItemSaver | undefined = () => undefined,
    split: SplitItem | null = null,
  ) {
    super(status);
    this.key = key;
    this.id = id;
    this.#create = create;
    this.#split = split;
    this.#events = events;
    this.#lookup = lookup;
  }

  /** 分割を送り終えていない、分割で作る項目か。 */
  get splitFrom(): string | null {
    return this.id === null ? (this.#split?.from ?? null) : null;
  }

  /** 送っている途中か。 */
  get inFlight(): boolean {
    return this.sending;
  }

  /** 分割の元の行の本文を入力した（分割を送るときに一緒に送る）。 */
  setFirst(text: string) {
    if (this.#split === null) return;
    if (this.sending) this.#lateFirst = text;
    else this.#split.first = text;
    this.markPending();
  }

  /** 分割の元の行の、まだ送っていない本文（分割を送らずにやめるとき、元の項目の保存に回す）。 */
  takeFirst(): string | null {
    const first = this.#lateFirst ?? this.#split?.first ?? null;
    this.#split = null;
    this.#lateFirst = undefined;
    return first;
  }

  edit(patch: ItemPatch) {
    this.#pending = mergePatch(this.#pending, patch);
    this.markPending();
  }

  /** まだ保存し終えていない、利用者が入力する欄。 */
  unsavedFields(): ("text" | "resolution_note")[] {
    const fields: ("text" | "resolution_note")[] = [];
    const has = (k: "text" | "resolutionNote") =>
      this.#pending[k] !== undefined || this.#sendingPatch?.[k] !== undefined;
    if (has("text")) fields.push("text");
    if (has("resolutionNote")) fields.push("resolution_note");
    return fields;
  }

  protected take(): ItemPatch | null {
    if (isEmptyPatch(this.#pending)) return null;
    if (this.id === null && this.#create === null && this.#split === null) {
      // 作るのに失敗して諦めた（reject）。作り直すと2つになりうるので、送らずに捨てて知らせる
      const p = this.#pending;
      this.#pending = {};
      this.#events.rejected?.(this.key, { kind: "not_found" }, p);
      return null;
    }
    // 作る前は、本文が入るまで送らない（本文の無い項目は作らない。空白だけも空とみなす）
    if (this.id === null && (this.#pending.text ?? "").trim() === "") return null;
    const p = this.#pending;
    this.#pending = {};
    this.#sendingPatch = p;
    return p;
  }

  protected async send(p: ItemPatch): Promise<Sent> {
    if (this.id === null) {
      const item = this.#split !== null ? await this.#sendSplit(this.#split, p.text!) : await this.#sendCreate(p.text!);
      this.id = item.id;
      this.#create = null;
      this.onCreated?.(this);
      this.#events.saved?.(this.key, item);
      // 作るのに使わなかった欄（解消メモ・状態）は、続けて保存する
      const { text: _text, ...rest } = p;
      if (!isEmptyPatch(rest)) this.#pending = mergePatch(rest, this.#pending);
      return isEmptyPatch(this.#pending) ? "done" : "again";
    }
    const r = await core.saveItem(this.id, p);
    this.#events.saved?.(this.key, r.item);
    return "done";
  }

  async #sendCreate(text: string): Promise<Item> {
    const create = this.#create!;
    const position = typeof create.position === "function" ? create.position() : create.position;
    return core.addItem(create.date, create.kind, text, position);
  }

  /**
   * 分割を送る。元の項目の保存を先に送り終え（失敗していれば、その失敗で再試行を待つ）、送る時点の
   * 元の行の本文 `first` と、この行の本文 `second` で分ける。元の行の本文が空なら、元の項目は変えずに、
   * この行をそのすぐ後に作る（元の行は、空のまま離れると消える。操作仕様書 2章）。
   */
  async #sendSplit(split: SplitItem, second: string): Promise<Item> {
    const orig = this.#lookup(split.from);
    if (orig !== undefined && !(await orig.flush())) {
      throw new CoreError(orig.failure ?? { kind: "storage", detail: "the original item still has unsaved input" });
    }
    const origId = orig === undefined ? split.from : orig.id;
    if (origId === null) throw new CoreError({ kind: "not_found" });
    if (split.first.trim() === "") {
      return core.addItem(split.date, split.kind, second, { at: "after", id: origId });
    }
    const r = await core.splitItem(origId, split.first, second);
    this.#split = null;
    this.#events.saved?.(split.from, r.original);
    // 送っている間に入力した元の行の本文は、元の項目の保存として送る
    if (this.#lateFirst !== undefined) {
      const late = this.#lateFirst;
      this.#lateFirst = undefined;
      (orig ?? this.#lookup(origId))?.edit({ text: late });
    }
    return r.created;
  }

  /**
   * 作る（`addItem`）ときは、保存先の問題（`storage`。トランザクションは取り消してある）と、まだ
   * 開いていない（`not_ready`）ときだけ再試行する。`internal` は書き込みが済んだかが分からず、
   * 作り直すと項目が2つになりうるので、再試行せずに知らせる（画面はボードを読み込み直す）。
   */
  protected retryable(e: CallError): boolean {
    if (this.id === null) return e.kind === "storage" || e.kind === "not_ready";
    return isRetryable(e);
  }

  protected putBack(p: ItemPatch) {
    this.#pending = mergePatch(p, this.#pending);
    // 分割を送っている間に入力した元の行の本文は、次に送る分割にまとめる
    if (this.#split !== null && this.#lateFirst !== undefined) {
      this.#split.first = this.#lateFirst;
      this.#lateFirst = undefined;
    }
  }

  protected reject(p: ItemPatch, e: CallError) {
    // 作るのに失敗したら、もう作らない（作れたかが分からず、作り直すと2つになりうるため。画面は
    // ボードを読み込み直し、作れていればその ID で続ける）
    if (this.id === null) {
      this.#create = null;
      this.#split = null;
      this.#lateFirst = undefined;
    }
    this.#events.rejected?.(this.key, e, p);
  }

  /** まだ保存し終えていない欄（送るのを待っている・送っている）。 */
  unsaved(): Set<ItemFieldName> {
    const fields = new Set<ItemFieldName>();
    for (const f of ["text", "resolutionNote", "status"] as const) {
      if (this.#pending[f] !== undefined || this.#sendingPatch?.[f] !== undefined) fields.add(f);
    }
    return fields;
  }

  protected settled() {
    this.#sendingPatch = null;
  }

  /** 送るのを待っている欄があるか。 */
  hasPending() {
    return !isEmptyPatch(this.#pending);
  }
}

/**
 * 項目の自動保存（docs/design/01-architecture.md 4.1「項目の保存」）。項目ごとに、利用者が変えた欄
 * だけを送る。`key` は、作った項目なら ID、まだ作っていない項目なら画面が付けた仮の名前
 * （`create` に渡したもの）。
 */
export class ItemSaves {
  readonly #status: SaveStatus;
  readonly #events: ItemSaveEvents;
  readonly #savers = new Map<string, ItemSaver>();

  constructor(status: SaveStatus, events: ItemSaveEvents = {}) {
    this.#status = status;
    this.#events = events;
  }

  readonly #lookup = (key: string) => this.#saver(key);

  #saver(key: string): ItemSaver {
    let s = this.#savers.get(key);
    if (s === undefined) {
      s = new ItemSaver(this.#status, key, key, null, this.#events, this.#lookup);
      this.#savers.set(key, s);
    }
    return s;
  }

  #add(s: ItemSaver) {
    if (this.#savers.has(s.key)) throw new Error(`item ${s.key} already exists`);
    // ID が決まったら、ID でも同じ待ち行列を使う（画面が ID で呼んでも、一度に1つの決まりを守る）
    s.onCreated = (created) => this.#savers.set(created.id!, created);
    this.#savers.set(s.key, s);
  }

  /** まだ作っていない項目を、本文 `text` で作る（作ったら `saved` に ID が届く）。 */
  create(key: string, item: NewItem, text: string) {
    if (this.#savers.has(key)) throw new Error(`item ${key} already exists`);
    const s = new ItemSaver(this.#status, key, null, item, this.#events, this.#lookup);
    this.#add(s);
    s.edit({ text });
  }

  /**
   * 項目 `from` を、本文 `first` と、新しい項目 `key` の本文 `second` に分ける（行の途中の Enter。
   * docs/design/01-architecture.md 4.1「自動保存」）。`from` の保存を送り終えてから、すぐに送る。
   * 保存先の問題で失敗したら、2行の本文を持ったまま再試行し、送る時点の本文で分ける（その間の元の行の
   * 入力も分割にまとめる）。作ったら `saved` に、元の項目と新しい項目が届く。
   */
  split(from: string, key: string, item: { date: LogicalDate; kind: ItemKind }, first: string, second: string) {
    if (this.#savers.has(key)) throw new Error(`item ${key} already exists`);
    const s = new ItemSaver(this.#status, key, null, null, this.#events, this.#lookup, { from, ...item, first });
    this.#add(s);
    s.edit({ text: second });
    void s.flush();
  }

  /** 分割を送り終えていない、`key` から分ける項目の待ち行列。 */
  #splitsFrom(key: string): ItemSaver[] {
    const id = this.idOf(key);
    return [...new Set(this.#savers.values())].filter(
      (s) => s.splitFrom !== null && (s.splitFrom === key || this.idOf(s.splitFrom) === id),
    );
  }

  /** 項目の ID（まだ作っていなければ `null`）。 */
  idOf(key: string): Id | null {
    const s = this.#savers.get(key);
    return s === undefined ? key : s.id;
  }

  /** 本文を入力した。分割を送り終えていない元の行なら、分割と一緒に送る。 */
  editText(key: string, text: string) {
    const split = this.#splitsFrom(key).at(-1);
    if (split !== undefined) split.setFirst(text);
    else this.#saver(key).edit({ text });
  }

  /**
   * まだ作っていない項目を、作らずにやめる（何も書かずに離れた行。操作仕様書 2章）。分割で作る項目
   * なら、まだ送っていない元の行の本文は、元の項目の保存として送る。作り始めている（送っている途中か、
   * 作り終えた）なら何もせず `false`（画面は作り終えるのを待って削除する）。
   */
  discard(key: string): boolean {
    const s = this.#savers.get(key);
    if (s === undefined) return true;
    if (s.id !== null || s.inFlight) return false;
    const from = s.splitFrom;
    const first = s.takeFirst();
    this.#savers.delete(key);
    void s.close();
    if (from !== null && first !== null) this.editText(from, first);
    return true;
  }

  /**
   * まだ保存し終えていない欄（読み込み直したときに、画面の値を残す欄）。分割を送り終えていない元の
   * 行は、本文を含む。
   */
  unsaved(key: string): Set<ItemFieldName> {
    const fields = this.#savers.get(key)?.unsaved() ?? new Set<ItemFieldName>();
    if (this.#splitsFrom(key).length > 0) fields.add("text");
    return fields;
  }

  /** 解消メモを入力した（空なら消す）。 */
  editResolutionNote(key: string, note: string) {
    this.#saver(key).edit({ resolutionNote: note });
  }

  /**
   * 完了・解消を切り替えた。`shown` は操作のときに画面に出ていた状態、`closedAt` は完了時刻
   * （未完了・未解消に戻すなら `null`。操作仕様書 2章のとおり画面が決める）。操作の時刻は今。
   */
  setClosed(key: string, shown: ItemStatus, closedAt: Millis | null) {
    this.#saver(key).edit({ status: { before: shown, closedAt, changedAt: Date.now() } });
  }

  /** その項目の、まだ保存していないものをすぐ保存する（フォーカスが外れたとき）。 */
  async flush(key: string) {
    await this.#savers.get(key)?.flush();
  }

  /**
   * その項目の保存を待ってから `op` を行う（削除・並べ替え・分割など、保存と順が入れ替わると困る
   * 操作）。保存に失敗して再試行している間は、`op` を行わずにその失敗の `CoreError` を投げる
   * （まだ保存していない入力の前の本文で分割・削除しないため）。作れていない項目なら `not_found`。
   */
  async run<T>(key: string, op: (id: Id) => Promise<T>): Promise<T> {
    // その項目から分ける分割も、先に送り終える（分割の前の本文で削除・並べ替えしないため）
    for (const split of this.#splitsFrom(key)) {
      if (!(await split.flush())) {
        throw new CoreError(split.failure ?? { kind: "internal", detail: "the item still has unsaved input" });
      }
    }
    const s = this.#savers.get(key);
    const done = (await s?.flush()) ?? true;
    if (s?.failure) throw new CoreError(s.failure);
    if (!done) throw new CoreError({ kind: "internal", detail: "the item still has unsaved input" });
    const id = this.idOf(key);
    if (id === null) throw new CoreError({ kind: "not_found" });
    return op(id);
  }

  /**
   * まだ保存し終えていない、利用者が入力する欄（入力中として知らせる欄に加える。
   * docs/design/01-architecture.md 4.1「書き込みが終わるまではフォーカスが無くても入力中」）。
   */
  unsavedFields(): EditingItem[] {
    const savers = [...new Set(this.#savers.values())];
    const fields = savers.flatMap((s) =>
      s.id === null ? [] : s.unsavedFields().map((field) => ({ itemId: s.id!, field })),
    );
    // 分割を送り終えていない元の項目の本文
    for (const s of savers) {
      const from = s.splitFrom === null ? null : this.idOf(s.splitFrom);
      if (from !== null && !fields.some((f) => f.itemId === from && f.field === "text")) {
        fields.push({ itemId: from, field: "text" });
      }
    }
    return fields;
  }

  /**
   * 保存を待っているものが無い項目の待ち行列を片付ける（表示している日を移ったときなど）。仮の名前で
   * 作った項目は、画面がその名前を使い続けうるので残す（片付けると、仮の名前を ID とみなしてしまう）。
   */
  prune() {
    for (const [key, s] of this.#savers) {
      if (!s.busy && !s.hasPending() && s.key === key && s.id === key) {
        void s.close();
        this.#savers.delete(key);
      }
    }
  }
}
