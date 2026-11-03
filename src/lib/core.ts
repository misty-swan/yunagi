// コアの呼び出し口（Tauri コマンドとイベント。docs/design/01-architecture.md 2章・4章 6）。
// 型は Rust から作った bindings.ts を使う。コマンドの名前と引数の名前は src-tauri/src/commands.rs と
// そろえる（引数は camelCase で渡し、Tauri が Rust の snake_case に読み替える）。

import { invoke, type InvokeArgs } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  EVENTS,
  type BackupFailed,
  type Board,
  type Changed,
  type CommandError,
  type Deleted,
  type Id,
  type Item,
  type ItemKind,
  type ItemOutcome,
  type LogicalDate,
  type Millis,
  type Position,
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

function toCallError(e: unknown): CallError {
  if (typeof e === "object" && e !== null && "kind" in e) return e as CommandError;
  return { kind: "bad_request", message: String(e) };
}

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
  /** 今日の論理日付。 */
  today: () => call<LogicalDate>("today"),
  getBoard: (date: LogicalDate) => call<Board>("get_board", { date }),
  addItem: (date: LogicalDate, kind: ItemKind, text: string, position: Position) =>
    call<Item>("add_item", { date, kind, text, position }),
  saveItemText: (id: Id, text: string) => call<ItemOutcome>("save_item_text", { id, text }),
  /** `closedAt` が数なら完了・解消（その時刻を完了時刻に）、`null` なら未完了・未解消に戻す。 */
  setItemClosed: (id: Id, closedAt: Millis | null) =>
    call<ItemOutcome>("set_item_closed", { id, closedAt }),
  saveResolutionNote: (id: Id, note: string) =>
    call<ItemOutcome>("save_resolution_note", { id, note }),
  deleteItem: (id: Id) => call<Deleted>("delete_item", { id }),
  restoreItem: (deleted: Deleted) => call<Item>("restore_item", { deleted }),
  moveItem: (id: Id, position: Position) => call<ItemOutcome>("move_item", { id, position }),
  splitItem: (id: Id, first: string, second: string) =>
    call<SplitResult>("split_item", { id, first, second }),
  getTextDoc: (doc: TextDoc, date: LogicalDate) => call<Snapshot>("get_text_doc", { doc, date }),
  /** `baseRev` はもとにした版。古ければ `{ result: "stale" }` で最新が返る。 */
  saveTextDoc: (doc: TextDoc, date: LogicalDate, body: string, baseRev: number) =>
    call<TextSave>("save_text_doc", { doc, date, body, baseRev }),
  /** 入力中の状態を知らせる（変わるたびに全体を知らせ直す）。 */
  reportUiState: (state: UiReport) => call<null>("report_ui_state", { state }),
};

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
};
