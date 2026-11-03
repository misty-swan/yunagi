// このファイルは src-tauri/src/bindings.rs が Rust の型から作る。手で書き換えない。
// 作り直すには：UPDATE_BINDINGS=1 cargo test -p yunagi-app --lib bindings

/**
 * 項目などの ID（ULID の 26 文字）。
 */
export type Id = string;

/**
 * 論理日付（`YYYY-MM-DD`）。TypeScript では、組み込みの `Date` と分けて `LogicalDate` と呼ぶ。
 */
export type LogicalDate = string;

/**
 * 時刻：UNIX 時刻のミリ秒（JavaScript の `Date.now()` と同じ）。
 */
export type Millis = number;

/**
 * 項目の種類。
 */
export type ItemKind = "todo" | "blocker";

/**
 * 項目の状態。TODO は `open` / `done` / `dropped`、障害は `open` / `resolved`。
 */
export type ItemStatus = "open" | "done" | "dropped" | "resolved";

/**
 * 取りやめた（`dropped`）理由。
 */
export type CloseReason = "dropped" | "split" | "returned_external";

/**
 * 項目を作ったもの。
 */
export type Origin = "human" | "agent" | "note" | "external";

/**
 * 項目の欄のうち、利用者が入力するもの（アーキテクチャ設計書 4.1「項目の入力中」）。
 */
export type ItemField = "text" | "resolution_note";

/**
 * ノートか振り返りか。
 */
export type TextDoc = "note" | "reflection";

/**
 * TODO か障害の1件（データモデル設計書 3章 item）。
 */
export type Item = { id: Id, date: LogicalDate, kind: ItemKind, text: string, status: ItemStatus, closeReason: CloseReason | null, 
/**
 * 並び順のキー。文字列の大小で並ぶ。
 */
sortKey: string, carryCount: number, carriedFromId: Id | null, splitFromId: Id | null, resolutionNote: string | null, origin: Origin, agentClientId: Id | null, externalRefId: Id | null, 
/**
 * 完了時刻（障害では解消時刻）。
 */
completedAt: Millis | null, createdAt: Millis, updatedAt: Millis, deletedAt: Millis | null, };

/**
 * 項目を入れる・動かす位置（同じ日の並びの中）。
 */
export type Position = { "at": "first" } | { "at": "last" } | { "at": "before", id: Id, } | { "at": "after", id: Id, };

/**
 * 画面からの項目の保存：利用者が変えた欄だけ（アーキテクチャ設計書 4.1「項目の保存」）。
 * 無い（`undefined`・`null`）欄には触れない。
 */
export type ItemFields = { 
/**
 * 本文。
 */
text?: string, 
/**
 * 障害の解消メモ（空なら消す）。
 */
resolutionNote?: string, 
/**
 * 状態（完了・解消の切り替え）。
 */
status?: StatusChange, };

/**
 * 項目の状態の保存（アーキテクチャ設計書 4.1「利用者とエージェントが項目を同じ状態にした行き違い」）。
 */
export type StatusChange = { 
/**
 * 変える前の値：利用者がこの保存で最初に状態を変えたときに、画面に出ていた状態。
 */
before: ItemStatus, 
/**
 * 数なら完了・解消にしてその時刻を完了時刻に、`null` なら未完了・未解消に戻す（完了時刻は
 * 画面が決める。操作仕様書 2章）。
 */
closedAt: Millis | null, 
/**
 * 利用者がこの保存で最初に状態を変えた操作の時刻。
 */
changedAt: Millis, };

/**
 * 項目の書き込みの結果。
 */
export type ItemOutcome = { 
/**
 * 実際に何かを変えたか。`false` なら何も書いていない。
 */
updated: boolean, 
/**
 * 操作の後の項目。
 */
item: Item, };

/**
 * 項目を2つに分けた結果。
 */
export type SplitResult = { 
/**
 * 操作の後の元の項目。
 */
original: Item, 
/**
 * 新しく作った項目（元の項目のすぐ後）。
 */
created: Item, };

/**
 * 削除した項目。削除の取り消し（Mod+Z）のために画面が持ち、そのまま `restore_item` に渡す。
 */
export type Deleted = { 
/**
 * 削除した項目の ID。
 */
id: Id, 
/**
 * 持ち越しで作った項目なら、削除の時点の持ち越し元の欄（変わっていたら戻せない）。
 */
source: SourceState | null, };

/**
 * 持ち越し先を削除した時点の、持ち越し元の項目の欄（`restore_item` が変わっていないかを確かめる）。
 */
export type SourceState = { id: Id, status: ItemStatus, text: string, resolutionNote: string | null, completedAt: Millis | null, };

/**
 * 持ち越し先：持ち越して作った項目と、その日付。
 */
export type CarriedTo = { id: Id, date: LogicalDate, };

/**
 * ボードの項目の1件：項目と、持ち越し先（持ち越し済みでなければ `null`）。
 */
export type BoardItem = { item: Item, carriedTo: CarriedTo | null, 
/**
 * エージェントが作った項目なら、作ったエージェントの識別子（`claude-code` など。agent_client の
 * `name`）。画面は本文の後ろに名前を添える（画面設計書 3章「各領域の表示」）。
 */
agentName: string | null, 
/**
 * エージェントによる未確認の変更がある（行頭の菱形）。
 */
unseen: boolean, };

/**
 * ノートの時刻付き見出し（`## 10:00 定例`）。
 */
export type TimeHeading = { 
/**
 * 見出しの行の番号（1始まり）。
 */
line: number, hour: number, minute: number, 
/**
 * 件名（無ければ空）。
 */
title: string, 
/**
 * `## ` の後ろの文字（例：`10:00 定例`）。
 */
text: string, };

/**
 * ボードのノート：見出しと長さだけ（本文は `get_text_doc` で読む）。
 */
export type NoteOutline = { headings: Array<TimeHeading>, 
/**
 * 本文の文字数。まだ書いていない日は 0。
 */
length: number, };

/**
 * その日のボード。
 */
export type Board = { date: LogicalDate, 
/**
 * TODO（並び順）。取りやめた TODO も入る。
 */
todos: Array<BoardItem>, 
/**
 * 障害（並び順）。
 */
blockers: Array<BoardItem>, 
/**
 * 振り返りの本文。まだ書いていない日は空。
 */
reflection: string, note: NoteOutline, };

/**
 * 件数（データモデル設計書 3章 item「件数の数え方」）。
 */
export type Counts = { 
/**
 * TODO 数（取りやめた TODO を除く）。
 */
todos: number, 
/**
 * 完了した TODO。
 */
done: number, 
/**
 * 残りのうち、未完了の TODO。
 */
leftTodos: number, 
/**
 * 残りのうち、未解消の障害。
 */
leftBlockers: number, 
/**
 * 解消した障害。
 */
resolved: number, 
/**
 * 前の日から持ち越してきた項目。
 */
carried: number, };

/**
 * サイドバーの Recent の1日。
 */
export type RecentDay = { date: LogicalDate, counts: Counts, };

/**
 * 画面の骨組み（サイドバー・上部のバー・凪メーター）に出す件数（画面設計書 3章「画面の骨組み」）。
 */
export type Overview = { 
/**
 * 表示している日の件数（凪メーター）。
 */
counts: Counts, 
/**
 * サイドバーの Recent（先頭はいつも今日の論理日付。新しい順に最大7日）。
 */
recent: Array<RecentDay>, 
/**
 * エージェントによる未確認の変更の数。
 */
unseenChanges: number, };

/**
 * ある時点の本文と版の番号（まだ書いていない日は空の本文と版 0）。
 */
export type Snapshot = { body: string, rev: number, };

/**
 * 画面からのノート・振り返りの保存の結果（アーキテクチャ設計書 4.1「保存の行き違いを防ぐ」）。
 */
export type TextSave = { "result": "saved", rev: number, updated: boolean, } | { "result": "stale", body: string, rev: number, };

/**
 * 画面が知らせる、入力中の状態（アーキテクチャ設計書 4.1）。変わるたびに全体を知らせ直す。
 *
 * 知らせはそれぞれ別のスレッドで処理されるので、続けて送ると届く順が入れ替わりうる。そこで
 * 画面の読み込みごとの `session` と、その中で増える `seq` を添え、コアは同じ `session` の古い
 * 知らせ（`seq` が最後に受け付けたもの以下）を捨てる。`src/lib/core.ts` が付ける。
 */
export type UiReport = { 
/**
 * 画面の読み込みごとに変わる文字列（読み込み直すと `seq` が 0 に戻るため）。
 */
session: string, 
/**
 * 同じ `session` の中で、送るたびに増える番号。
 */
seq: number, 
/**
 * 入力中の項目の欄（フォーカスがあり最後の入力から 10 秒以内か IME の変換中、
 * またはまだ保存していない入力の書き込みを待っている欄）。
 */
editingItems: Array<EditingItem>, 
/**
 * ノート・振り返りの守る行。守る行の無いエディタは入れない（入れても無いものとして扱う）。
 */
editors: Array<EditorLines>, };

/**
 * 入力中の項目の欄。
 */
export type EditingItem = { itemId: Id, field: ItemField, };

/**
 * ノート・振り返りの守る行。
 */
export type EditorLines = { doc: TextDoc, date: LogicalDate, 
/**
 * 行番号を数えた本文（最後に保存した本文）の版。
 */
rev: number, 
/**
 * 守る行の番号（1始まり）。
 */
lines: Array<number>, };

/**
 * データベースを開く処理の状態（起動直後に画面が `startup_status` で問い合わせ、変わると
 * `startup` イベントでも届く）。
 */
export type Startup = { "state": "opening" } | { "state": "ready" } | { "state": "not_found", 
/**
 * 探した `yunagi.db` のパス。
 */
path: string, 
/**
 * 保存先。
 */
dataDir: string, 
/**
 * 保存先を変えているときの、それを記録した `config.json`。
 */
configuredIn: string | null, 
/**
 * 保存先のフォルダそのものがあるか。
 */
dataDirExists: boolean, } | { "state": "failed", reason: StartupFailure, detail: string, };

/**
 * データベースを開けなかった理由。
 */
export type StartupFailure = "default_dir" | "lock_busy" | "lock" | "config" | "already_exists" | "storage";

/**
 * `changed` イベント：コアでデータが変わった。画面は表示中のデータを読み込み直す
 * （アーキテクチャ設計書 4章 6）。
 */
export type Changed = { by: ChangeSource, };

/**
 * 変えたもの。
 */
export type ChangeSource = "screen" | "agent";

/**
 * `today-changed` イベント：今日の論理日付が変わった（切り替わり時刻を越えた）。
 */
export type TodayChanged = { today: LogicalDate, };

/**
 * `backup-failed` イベント：定期バックアップを作れなかった。`detail` はログ用の文（英語）。
 */
export type BackupFailed = { detail: string, };

/**
 * コマンドの失敗。`kind` で見分ける。画面に出す文は画面が `kind` から決める
 * （Display の文をそのまま使わない。README「エラーの表示」）。
 */
export type CommandError = { "kind": "not_ready" } | { "kind": "invalid_state" } | { "kind": "not_found" } | { "kind": "invalid_argument", reason: InvalidReason, } | { "kind": "conflict" } & Conflict | { "kind": "restore_refused" } & RestoreRefused | { "kind": "storage", detail: string, } | { "kind": "internal", detail: string, };

/**
 * 引数の誤りの種類。
 */
export type InvalidReason = "wrong_kind" | "dropped" | "empty_text" | "multiline_text" | "empty_note" | "multiline_note" | "invalid_position" | "not_linked_external" | "note_line" | "invalid_line";

/**
 * 受け付けられない状態の理由。
 */
export type Conflict = { "reason": "carried_over", carriedToId: Id, carriedToDate: LogicalDate, } | { "reason": "user_editing" } | { "reason": "note_exists", resolutionNote: string, };

/**
 * 削除を戻せない理由。
 */
export type RestoreRefused = { "reason": "source_changed" } | { "reason": "source_carried_over", carriedToId: Id, carriedToDate: LogicalDate, };

/** コアが出すイベントの名前（src-tauri/src/lib.rs）。 */
export const EVENTS = {
  startup: "startup",
  changed: "changed",
  todayChanged: "today-changed",
  backupFailed: "backup-failed",
} as const;
