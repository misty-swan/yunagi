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
unseen: boolean, 
/**
 * ノートから送った項目なら、送り元の行（持ち越し・分割の元へさかのぼる）。画面は行末の
 * 「← NOTE」からその行へ移る（画面設計書 3章「各領域の表示」）。
 */
noteSource: NoteSource | null, };

/**
 * 項目の送り元の行（データモデル設計書 3章 note_send）。
 */
export type NoteSource = { 
/**
 * 送り元のノートの日。
 */
date: LogicalDate, 
/**
 * 保存済みの本文での、送り元の行（1始まり）。
 */
line: number, 
/**
 * 送った記録の送り先の項目（持ち越し・分割で作った項目では、元の項目）。画面は、エディタの
 * 本文の送信済みの印（`note_marks`）からこの項目の行を探し、無ければ `line` を使う。
 */
itemId: Id, };

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
reflection: string, note: NoteOutline, 
/**
 * 振り返りの版（`get_text_doc` の `rev`。まだ書いていない日は 0）。画面は、これが進んだときだけ
 * 振り返りの本文を読み直して取り込む。
 */
reflectionRev: number, 
/**
 * ノートの版（同上）。
 */
noteRev: number, };

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
 * 週のカードの1日：日付とその日の件数。
 */
export type DayCounts = { date: LogicalDate, counts: Counts, };

/**
 * Daily notes の1日：日付、件数、振り返りの本文（GOOD / STUCK の1行は画面が取り出す）。
 */
export type DailyEntry = { date: LogicalDate, counts: Counts, reflection: string, };

/**
 * プレビューのノートの1区間：時刻付き見出しと、その下の最初の行。
 */
export type NotePreview = { hour: number, minute: number, title: string, lines: Array<string>, };

/**
 * 選んだ日のプレビュー。
 */
export type DayPreview = { counts: Counts, reflection: string, note: Array<NotePreview>, };

/**
 * ふりかえり画面の内容（画面設計書 9章。コアの `domain::archive`）。
 */
export type Archive = { 
/**
 * 選んだ日。
 */
date: LogicalDate, 
/**
 * 今日の論理日付。
 */
today: LogicalDate, 
/**
 * 選んだ日を含む週（月〜金）の日ごとの件数。
 */
week: Array<DayCounts>, 
/**
 * 週の合計。
 */
weekTotal: Counts, 
/**
 * 記録のある直近の日（新しい順）。
 */
daily: Array<DailyEntry>, preview: DayPreview, };

/**
 * ある時点の本文と版の番号（まだ書いていない日は空の本文と版 0）。
 */
export type Snapshot = { body: string, rev: number, };

/**
 * 画面からのノート・振り返りの保存の結果（アーキテクチャ設計書 4.1「保存の行き違いを防ぐ」）。
 */
export type TextSave = { "result": "saved", rev: number, updated: boolean, } | { "result": "stale", body: string, rev: number, };

/**
 * 送信済みの印を付けるノートの行（`note_marks`。同じ行に2つ付くこともある）。
 */
export type NoteMark = { 
/**
 * 行の番号（1始まり。渡した本文で数える）。
 */
line: number, 
/**
 * 送り先の種類（「→ TODO」「→ BLOCKER」）。
 */
kind: ItemKind, 
/**
 * 送り先の項目。
 */
itemId: Id, };

/**
 * 送らなかった行の理由（データモデル設計書 3章 note_send「送らない行」）。
 */
export type SkipReason = "heading" | "no_text" | "in_code" | "already_sent";

/**
 * 送った1行。
 */
export type SentLine = { line: number, 
/**
 * 作った項目。
 */
item: Item, };

/**
 * 送らなかった1行。
 */
export type SkippedLine = { line: number, reason: SkipReason, };

/**
 * 行送り（`send_note_lines`）の結果。
 */
export type SendResult = { "result": "sent", rev: number, 
/**
 * 送った行（上から順）。
 */
sent: Array<SentLine>, 
/**
 * 送らなかった行（上から順）。
 */
skipped: Array<SkippedLine>, } | { "result": "stale", body: string, rev: number, };

/**
 * 持ち越しの画面を、いつ出すか（画面設計書 5章）。
 * - `show`：まだ後回しにしていない。表示したときに出す。
 * - `reshow_on_focus`：後回しにしたが、まだ再表示していない。次にアプリを前面に出したとき
 *   1回だけ出し、`mark_carryover_reshown` で記録する。
 * - `on_request`：後回しにして、再表示も済んだ。自動では出さず、パレットの「持ち越しを開く」で開く。
 */
export type CarryoverPrompt = "show" | "reshow_on_focus" | "on_request";

/**
 * 今日の持ち越しが残っているときの状態（`carryover_status`）。
 */
export type CarryoverStatus = { 
/**
 * 前回の日。
 */
from: LogicalDate, prompt: CarryoverPrompt, };

/**
 * 持ち越しの画面に出す1件。
 */
export type CarryoverCandidate = { 
/**
 * 前回の日の項目（未完了の TODO か、未解消の障害）。
 */
item: Item, 
/**
 * 見直しの選択肢（分割する／やめる）を添えるか：持ち越し回数が見直しの回数（設定）以上の TODO。
 */
review: boolean, };

/**
 * 持ち越しの画面の中身（`carryover_candidates`）。
 */
export type CarryoverList = { 
/**
 * 前回の日。確定（`confirm_carryover`）にそのまま渡す。
 */
from: LogicalDate, 
/**
 * 前回の日の並び順。
 */
items: Array<CarryoverCandidate>, 
/**
 * 見直しの回数（設定）。持ち越し回数がこれ以上の印を `--warn` にする（障害にも。デザインガイド 6章）。
 */
reviewThreshold: number, };

/**
 * 持ち越しの画面のボタン。
 */
export type CarryoverSelection = { "select": "all" } | { "select": "selected", ids: Array<Id>, } | { "select": "none" };

/**
 * 見直しの選択。
 */
export type CarryoverReview = { "review": "drop" } | { "review": "return_external" } | { "review": "split", children: Array<string>, };

/**
 * 見直しを選んだ TODO と、その選択。
 */
export type CarryoverReviewChoice = { id: Id, review: CarryoverReview, };

/**
 * 持ち越しの確定の中身（`confirm_carryover`）。
 */
export type CarryoverDecision = { 
/**
 * 画面に出していた前回の日（`CarryoverList` の `from`）。
 */
from: LogicalDate, selection: CarryoverSelection, 
/**
 * 見直しを選んだ TODO（どのボタンで確定しても反映する）。同じ項目が2回あれば後のものを使う。
 */
reviews: Array<CarryoverReviewChoice>, };

/**
 * 持ち越しを確定した結果。確定は Mod+Z で取り消せないので、画面は取り消しの履歴に持たない
 * （操作仕様書 2章「取り消し・やり直し」）。
 */
export type CarryoverConfirmed = { 
/**
 * 今日に作った項目（持ち越した項目と分割の子）。今日の並び順。
 */
created: Array<Item>, 
/**
 * 見直しで取りやめた、前回の日の項目（操作の後）。
 */
closed: Array<Item>, };

/**
 * 検索結果の種類（画面設計書 6章の英字ラベル TODO / BLOCKER / NOTE / REFLECTION）。
 */
export type SearchKind = "todo" | "blocker" | "note" | "reflection";

/**
 * 検索結果の1件（データモデル設計書 4.1）。一致した部分の切り出しは画面が行う。
 */
export type SearchHit = { kind: SearchKind, 
/**
 * 元の行：TODO・障害は項目の id、ノート・振り返りはその日の日付（`YYYY-MM-DD`）。
 */
refId: string, date: LogicalDate, 
/**
 * 本文の全体（項目は1行の本文、ノート・振り返りは本文すべて）。
 */
body: string, 
/**
 * 解消済みの障害の解消メモ（無ければ `null`）。一致したのが本文か解消メモかは画面が見分ける。
 */
resolutionNote: string | null, 
/**
 * 項目の持ち越し先。持ち越し済みでない項目と、ノート・振り返りは `null`。
 */
carriedTo: CarriedTo | null, };

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
export type Startup = { "state": "opening" } | { "state": "ready" } | { "state": "restored", 
/**
 * 退避した、それまでのデータベース（`backups/damaged-<秒>.db`）。無かったときは `None`。
 */
setAside: string | null, } | { "state": "not_found", 
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
dataDirExists: boolean, } | { "state": "failed", reason: StartupFailure, detail: string, 
/**
 * 開こうとした `yunagi.db` のパス（保存先が分かったとき）。
 */
database: string | null, 
/**
 * マイグレーション前のバックアップ（`migration_failed`・`migration_restore_failed` のとき。
 * 適用前のデータがある）。
 */
backup: string | null, 
/**
 * `config.json` のパス（`config` のとき）。
 */
config: string | null, };

/**
 * データベースを開けなかった理由。
 */
export type StartupFailure = "default_dir" | "lock_busy" | "lock" | "config" | "already_exists" | "migration_failed" | "migration_restore_failed" | "unsupported_schema" | "corrupted" | "storage";

/**
 * `backups/` にある1つのバックアップ（`list_backups`）。
 */
export type BackupEntry = { 
/**
 * ファイルのパス。戻すときに `restore_at_startup` に渡す。
 */
path: string, kind: BackupKind, 
/**
 * 作った時刻（秒まで）。
 */
createdAt: Millis, };

/**
 * バックアップの種類（アーキテクチャ設計書 6.2）。
 */
export type BackupKind = { "type": "daily", date: LogicalDate, } | { "type": "pre_migration", from: number, to: number, } | { "type": "pre_restore" };

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
 * テーマ（画面設計書 7章「表示」）。
 */
export type Theme = "system" | "light" | "dark";

/**
 * ノートの書体（画面設計書 7章「表示」、デザインガイド 4章）。
 */
export type NoteFont = "body" | "mono";

/**
 * 整理モードのレイアウト（画面設計書 3章「レイアウト」）。
 */
export type Layout = "A" | "B" | "C";

/**
 * 画面の言語。
 */
export type Locale = "ja" | "en";

/**
 * 画面に出す設定（既定値を当てた後のもの。データモデル設計書 3章 setting）。時刻は `HH:MM`。
 */
export type Settings = { 
/**
 * 日付の切り替わり時刻（既定 `04:00`）。
 */
dayBoundary: string, 
/**
 * 凪メーターの始業（既定 `09:00`）。
 */
meterStart: string, 
/**
 * 凪メーターの終業（既定 `19:00`）。始業より後。
 */
meterEnd: string, 
/**
 * 画面の言語。設定が無ければ `null`（OS の言語に合わせる）。
 */
locale: Locale | null, 
/**
 * 更新の自動確認（既定 `true`）。
 */
updateCheck: boolean, theme: Theme, 
/**
 * フォントサイズ（px。13〜19、既定 15）。
 */
fontSize: number, noteFont: NoteFont, layout: Layout, 
/**
 * 持ち越し見直しの回数（1 以上、既定 3）。
 */
carryoverReviewThreshold: number, 
/**
 * サイドバーの開閉（広いウィンドウの整理モードでの状態。既定は開く）。
 */
sidebarOpen: boolean, 
/**
 * ショートカットの割当：操作の名前から割当の文字列へ。初期割当から変えたものだけ
 * （データモデル設計書 3章 setting）。
 */
shortcuts: { [key in string]: string }, 
/**
 * Vim モード（既定 `false`）。
 */
vimMode: boolean, 
/**
 * Vim モードの挿入モードで、直前の入力状態に戻す（既定 `true`）。
 */
vimImeRestore: boolean, };

/**
 * 設定の保存：変える欄だけ（無い欄には触れない）。`meterStart`・`meterEnd` は、片方だけでも、
 * もう片方は今の値を使う。
 */
export type SettingsPatch = { dayBoundary?: string, meterStart?: string, meterEnd?: string, locale?: Locale, updateCheck?: boolean, theme?: Theme, fontSize?: number, noteFont?: NoteFont, layout?: Layout, carryoverReviewThreshold?: number, sidebarOpen?: boolean, 
/**
 * 割当の全体（欄ごと置き換える。初期割当と同じものは入れない）。
 */
shortcuts?: { [key in string]: string }, vimMode?: boolean, vimImeRestore?: boolean, };

/**
 * クラウド同期のサービス（保存先の警告に使う。画面設計書 7章）。
 */
export type CloudService = "i_cloud_drive" | "dropbox" | "one_drive" | "google_drive";

/**
 * 保存先の変更の結果（`change_data_dir`）。
 */
export type DataDirChanged = { 
/**
 * 移した後に消せなかった、元の保存先のファイル・フォルダ（データは新しい保存先に移っている。
 * 消すかどうかは利用者に任せる）。
 */
leftovers: Array<string>, };

/**
 * IME の切替に使う仕組み（操作仕様書 4.4。`src-tauri/src/ime.rs`）。
 */
export type ImeMethod = "fcitx5" | "ibus" | "macos_input_source" | "windows_imm";

/**
 * この環境で IME の切替ができるか（`ime_status`）。できないとき（Linux で fcitx5 も IBus も動いていない
 * など）、設定画面が制約を表示する（画面設計書 7章「操作」）。
 */
export type ImeStatus = { available: boolean, method: ImeMethod | null, };

/**
 * OS 全体のショートカット（Yunagi を前面に呼び出す）の、いまの登録の状態（`set_global_shortcut`）。
 * 設定画面が、登録できなかった理由の表示に使う（操作仕様書 1章「割当の変更」）。
 */
export type GlobalShortcutState = "off" | "registered" | "taken" | "failed" | "unavailable";

/**
 * OS 全体のショートカットの登録の結果。
 */
export type GlobalShortcutStatus = { state: GlobalShortcutState, 
/**
 * Linux の Wayland のセッションか。Wayland では、登録できたように見えても効かないことがあり
 * （登録の成否を OS から知る手段が無い）、設定画面は常に案内を出す（アーキテクチャ設計書 7章）。
 */
wayland: boolean, };

/**
 * コマンドの失敗。`kind` で見分ける。画面に出す文は画面が `kind` から決める
 * （Display の文をそのまま使わない。README「エラーの表示」）。
 */
export type CommandError = { "kind": "not_ready" } | { "kind": "invalid_state" } | { "kind": "not_found" } | { "kind": "invalid_argument", reason: InvalidReason, } | { "kind": "conflict" } & Conflict | { "kind": "restore_refused" } & RestoreRefused | { "kind": "storage", detail: string, } | { "kind": "rejected", reason: RejectReason, } | { "kind": "export_failed", replaced: number, detail: string, } | { "kind": "put_back_failed", setAside: string, detail: string, } | { "kind": "internal", detail: string, };

/**
 * 引数の誤りの種類。
 */
export type InvalidReason = "wrong_kind" | "dropped" | "empty_text" | "multiline_text" | "empty_note" | "multiline_note" | "invalid_position" | "not_linked_external" | "note_line" | "invalid_line" | "meter_hours" | "font_size" | "review_threshold" | "shortcuts" | "time_of_day";

/**
 * 選んだバックアップ・保存先を使えない理由。
 */
export type RejectReason = "invalid_path" | "not_a_directory" | "no_database" | "unreadable" | "corrupted" | "not_yunagi" | "newer_schema" | "same_directory" | "inside_backups" | "target_has_data";

/**
 * 受け付けられない状態の理由。
 */
export type Conflict = { "reason": "carried_over", carriedToId: Id, carriedToDate: LogicalDate, } | { "reason": "user_editing" } | { "reason": "note_exists", resolutionNote: string, } | { "reason": "previous_day_changed", previousDay: LogicalDate | null, } | { "reason": "carryover_done" };

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
