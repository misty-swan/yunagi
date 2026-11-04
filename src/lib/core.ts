// コアの呼び出し口（Tauri コマンドとイベント。docs/design/01-architecture.md 2章・4章 6）。
// 型は Rust から作った bindings.ts を使う。コマンドの名前と引数の名前は src-tauri/src/commands.rs と
// そろえる（引数は camelCase で渡し、Tauri が Rust の snake_case に読み替える）。

import { invoke, type InvokeArgs } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  EVENTS,
  type Archive,
  type BackupEntry,
  type BackupFailed,
  type Board,
  type CarryoverConfirmed,
  type CarryoverDecision,
  type CarryoverList,
  type CarryoverStatus,
  type Changed,
  type CloudService,
  type CommandError,
  type DataDirChanged,
  type Deleted,
  type Id,
  type GlobalShortcutStatus,
  type ImeStatus,
  type Item,
  type ItemFields,
  type ItemKind,
  type ItemOutcome,
  type LogicalDate,
  type NoteMark,
  type Overview,
  type Position,
  type SearchHit,
  type SendResult,
  type Settings,
  type SettingsPatch,
  type ShowRequested,
  type Snapshot,
  type SplitResult,
  type Startup,
  type TextDoc,
  type TextSave,
  type TodayChanged,
  type UiReport,
} from "./bindings";

/**
 * コマンドの失敗。コアが返したもの（`CommandError`）か、引数の形が誤っていて Tauri が読めなかった
 * もの（`bad_request`。画面の不具合）。
 */
export type CallError = CommandError | { kind: "bad_request"; message: string };

/** コマンドが失敗したときに投げる。中身は `error`。 */
export class CoreError extends Error {
  readonly error: CallError;

  constructor(error: CallError) {
    super(`core command failed: ${error.kind}`);
    this.name = "CoreError";
    this.error = error;
  }
}

/** `core` のコマンドが投げたものの中身（`CoreError` でなければ `bad_request` として扱う）。 */
export function errorOf(e: unknown): CallError {
  return e instanceof CoreError ? e.error : { kind: "bad_request", message: String(e) };
}

function toCallError(e: unknown): CallError {
  if (typeof e === "object" && e !== null && "kind" in e) return e as CommandError;
  return { kind: "bad_request", message: String(e) };
}

/** この画面の読み込みの印（読み込み直すと変わる。読み込みごとに違えばよく、秘密ではない）。 */
const PAGE_SESSION = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
let reportSeq = 0;

async function call<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw new CoreError(toCallError(e));
  }
}

/** コアのコマンド。失敗すると `CoreError` を投げる。 */
export const core = {
  /** データベースを開く処理の状態。`onStartup` を聞き始めてから呼ぶ。 */
  startupStatus: () => call<Startup>("startup_status"),
  /** データベースが見つからないときに、新しく作って開く。 */
  createDatabase: () => call<Startup>("create_database"),
  /** 見つからなかった・開けなかったときに、もう一度開く。 */
  retryOpen: () => call<Startup>("retry_open"),
  /** 見つからなかったときに、データベースのある既存のフォルダ `dir` を保存先にして開く。 */
  chooseDataDir: (dir: string) => call<Startup>("choose_data_dir", { dir }),
  /** 見つからなかった・壊れていて開けなかったときに、バックアップ `backup` で置き換えて開く。 */
  restoreAtStartup: (backup: string) => call<Startup>("restore_at_startup", { backup }),
  /** バックアップから戻した結果（`restored`）を、利用者が確かめた。 */
  acknowledgeRestore: () => call<Startup>("acknowledge_restore"),
  /** 保存先の `backups/` にあるバックアップの一覧（新しいものが先）。 */
  listBackups: () => call<BackupEntry[]>("list_backups"),
  /** 今日の論理日付。 */
  today: () => call<LogicalDate>("today"),
  getBoard: (date: LogicalDate) => call<Board>("get_board", { date }),
  /** 画面の骨組み（サイドバー・上部のバー・凪メーター）に出す件数。`date` は表示している日。 */
  getOverview: (date: LogicalDate) => call<Overview>("get_overview", { date }),
  /**
   * ふりかえり（S-08）の内容：選んだ日 `date`（`null` なら、今日より前で記録のある最も新しい日。無ければ
   * 今日。今日より後は今日になる）を含む週の件数、記録のある直近の日、選んだ日のプレビュー。
   */
  getArchive: (date: LogicalDate | null) => call<Archive>("get_archive", { date }),
  addItem: (date: LogicalDate, kind: ItemKind, text: string, position: Position) =>
    call<Item>("add_item", { date, kind, text, position }),
  /**
   * 項目の、利用者が変えた欄だけを保存する（無い欄には触れない）。状態を変えたときは、変える前の
   * 値と最初の操作の時刻を添える（エージェントとの行き違いの判定に使う）。画面からは、自動保存
   * （`autosave.svelte.ts` の `ItemSaves`）を通して呼ぶ。
   */
  saveItem: (id: Id, fields: ItemFields) => call<ItemOutcome>("save_item", { id, fields }),
  deleteItem: (id: Id) => call<Deleted>("delete_item", { id }),
  restoreItem: (deleted: Deleted) => call<Item>("restore_item", { deleted }),
  moveItem: (id: Id, position: Position) => call<ItemOutcome>("move_item", { id, position }),
  splitItem: (id: Id, first: string, second: string) =>
    call<SplitResult>("split_item", { id, first, second }),
  getTextDoc: (doc: TextDoc, date: LogicalDate) => call<Snapshot>("get_text_doc", { doc, date }),
  /** `baseRev` はもとにした版。古ければ `{ result: "stale" }` で最新が返る。 */
  saveTextDoc: (doc: TextDoc, date: LogicalDate, body: string, baseRev: number) =>
    call<TextSave>("save_text_doc", { doc, date, body, baseRev }),
  /**
   * ノートの `from` 行目から `to` 行目まで（1始まり。両端を含む）を、1行ごとに `kind` の項目として送る。
   * `body` はエディタの本文、`baseRev` はそのもとにした版。古ければ `{ result: "stale" }` で最新が返る。
   */
  sendNoteLines: (date: LogicalDate, body: string, baseRev: number, from: number, to: number, kind: ItemKind) =>
    call<SendResult>("send_note_lines", { date, body, baseRev, from, to, kind }),
  /** 本文 `body`（エディタの本文）で、送信済みの印を付ける行。 */
  noteMarks: (date: LogicalDate, body: string) => call<NoteMark[]>("note_marks", { date, body }),
  /** 今日の持ち越しの状態。終えたか、持ち越すものが無ければ `null`。 */
  carryoverStatus: () => call<CarryoverStatus | null>("carryover_status"),
  /** 持ち越しの画面に出す項目。前回の日が無ければ `null`。 */
  carryoverCandidates: () => call<CarryoverList | null>("carryover_candidates"),
  /** 持ち越しを後回しにした（Esc）ことを記録する。 */
  postponeCarryover: () => call<null>("postpone_carryover"),
  /** 後回しにした持ち越しを、前面に出したときに再表示したことを記録する。記録したら `true`（そのときだけ出す）。 */
  markCarryoverReshown: () => call<boolean>("mark_carryover_reshown"),
  /** 持ち越しを確定する。 */
  confirmCarryover: (decision: CarryoverDecision) =>
    call<CarryoverConfirmed>("confirm_carryover", { decision }),
  /** 設定（既定値を当てた後のもの）。 */
  getSettings: () => call<Settings>("get_settings"),
  /** 設定を保存する（変える欄だけ）。保存した後の設定を返す。 */
  saveSettings: (patch: SettingsPatch) => call<Settings>("save_settings", { patch }),
  /** いまの保存先のフォルダ。 */
  getDataDir: () => call<string>("get_data_dir"),
  /** 保存先の候補 `dir` がクラウド同期フォルダ（の中）と判定できるか（判定できなければ `null`）。 */
  checkDataDir: (dir: string) => call<CloudService | null>("check_data_dir", { dir }),
  /** 保存先を `dir` に変える（データベースと `backups/` を移す）。 */
  changeDataDir: (dir: string) => call<DataDirChanged>("change_data_dir", { dir }),
  /** データベースのある既存のフォルダ `dir` を、データを移さずに保存先にする。開くデータベースが変わる。 */
  useDataDir: (dir: string) => call<null>("use_data_dir", { dir }),
  /** バックアップ `backup`（`listBackups` の `path`）から復元する。画面は、復元の後に読み込み直す。 */
  restoreBackup: (backup: string) => call<null>("restore_backup", { backup }),
  /** `from` から `to` までの日を、日付ごとの Markdown としてフォルダ `dir` に書き出す。書いたファイルのパスを返す。 */
  exportMarkdown: (from: LogicalDate, to: LogicalDate, dir: string) =>
    call<string[]>("export_markdown", { from, to, dir }),
  /** 全日付を `text` で探す（パレット S-04）。日付の新しい順に `limit` 件まで。 */
  search: (text: string, limit: number) => call<SearchHit[]>("search", { text, limit }),
  /**
   * 入力中の状態を知らせる（変わるたびに全体を知らせ直す）。届く順が入れ替わっても古いものが
   * 残らないよう、この画面の読み込みの印と、送るたびに増える番号を付ける。
   */
  reportUiState: (state: Omit<UiReport, "session" | "seq">) =>
    call<null>("report_ui_state", { state: { ...state, session: PAGE_SESSION, seq: ++reportSeq } }),
  /** この環境で IME（日本語入力）の切替ができるか（設定画面の制約の表示。操作仕様書 4.4 の 4）。 */
  imeStatus: () => call<ImeStatus>("ime_status"),
  /** Vim の挿入モードを抜けた：IME を英数入力に切り替える。切替は待たずに返る（失敗しても失敗にならない）。 */
  imeLeaveInsert: () => call<null>("ime_leave_insert"),
  /** Vim の挿入モードに入った：`restore` なら抜けた時点の入力状態に戻す（どちらでも、覚えていた状態は捨てる）。 */
  imeEnterInsert: (restore: boolean) => call<null>("ime_enter_insert", { restore }),
  /**
   * OS 全体のショートカット（Yunagi を前面に呼び出す）の割当を `binding` にして登録し直す（`null` は外す）。
   * 登録できなかった理由は、失敗にせず、返す状態で知らせる。
   */
  setGlobalShortcut: (binding: string | null) => call<GlobalShortcutStatus>("set_global_shortcut", { binding }),
  /**
   * ノートモードで前面に出す頼み（`yunagi show --note`）が、まだ渡っていなければ受け取る（`true`。受け取ると忘れる）。
   * 画面が準備できたとき（起動した直後は、頼みが画面の準備より先に届く）と、`show-requested` を受けたときに呼ぶ。
   */
  takeShowNote: () => call<boolean>("take_show_note"),
};

/** イベントを聞き始められなかったとき、やり直すまでの時間。 */
export const LISTEN_RETRY_MS = 1000;

/**
 * イベントを聞き始める。聞き始められなければ、`stopped()` になるまでやり直す（失敗するたびに
 * `onError`）。聞き始めたら、聞くのをやめる関数で解決する。聞き始めた時点で `stopped()` なら、すぐに
 * やめて `null` で解決する。
 */
export async function keepListening(
  register: () => Promise<UnlistenFn>,
  stopped: () => boolean,
  onError?: (e: unknown) => void,
): Promise<UnlistenFn | null> {
  while (!stopped()) {
    try {
      const u = await register();
      if (!stopped()) return u;
      u();
      return null;
    } catch (e) {
      onError?.(e);
      await new Promise((resolve) => setTimeout(resolve, LISTEN_RETRY_MS));
    }
  }
  return null;
}

/** コアのイベントを聞く。戻り値の関数で聞くのをやめる。 */
export const coreEvents = {
  onStartup: (cb: (p: Startup) => void): Promise<UnlistenFn> =>
    listen<Startup>(EVENTS.startup, (e) => cb(e.payload)),
  onChanged: (cb: (p: Changed) => void): Promise<UnlistenFn> =>
    listen<Changed>(EVENTS.changed, (e) => cb(e.payload)),
  onTodayChanged: (cb: (p: TodayChanged) => void): Promise<UnlistenFn> =>
    listen<TodayChanged>(EVENTS.todayChanged, (e) => cb(e.payload)),
  onBackupFailed: (cb: (p: BackupFailed) => void): Promise<UnlistenFn> =>
    listen<BackupFailed>(EVENTS.backupFailed, (e) => cb(e.payload)),
  onShowRequested: (cb: (p: ShowRequested) => void): Promise<UnlistenFn> =>
    listen<ShowRequested>(EVENTS.showRequested, (e) => cb(e.payload)),
};
