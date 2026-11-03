// 画面の文言（デザインガイド 7章）。
//
// - 日本語と英語は同じ情報量にそろえ、文言キー（このファイルの `Messages` の名前）で管理する。
// - ラベル（ナビ、凪メーター、キーの案内、件数、日付の表示など）は、日本語の画面でも英語で書く
//   （`labels`。日英で共通）。文（状態の説明、操作ボタン、エラーなど）は各言語で書く。
// - 短く、命令形を避け、状態を淡々と伝える。感嘆符・絵文字・励ましの言葉は使わない。

import type { CloseReason, RejectReason, StartupFailure } from "./bindings";
import { prefs, type Lang } from "./prefs.svelte";

/** 日英で共通の英字ラベル。 */
const labels = {
  nav: { today: "Today", archive: "Archive", agents: "Agents", settings: "Settings" },
  recent: "Recent",
  board: "Board",
  note: "Note",
  today: "Today",
  savedLocally: "Saved locally",
  meter: {
    left: "left",
    todo: "TODO",
    blockers: "Blockers",
    done: "Done",
    carried: "Carried",
    nagi: "NAGI",
    dayEnd: "Day end",
    notYet: "Not yet",
  },
  hint: {
    search: "Search",
    note: "Note",
    board: "Board",
    done: "Done",
    day: "Day",
    archive: "Archive",
    sidebar: "Sidebar",
    toTodo: "To TODO",
    toBlockers: "To Blockers",
    heading: "Time heading",
  },
  /** ノートの右の、時刻付き見出しの一覧（画面設計書 4章）。 */
  outline: "Outline",
  /** Outline の下の、TODO・障害へ送った行数（画面設計書 4章）。 */
  sent: "Sent",
  /** 項目の行末の、送り元のノートの行への印（画面設計書 3章「各領域の表示」）。 */
  fromNote: "← NOTE",
  /** 整理モードの領域の見出し（画面設計書 3章「各領域の表示」）。 */
  region: { todo: "TODO", blockers: "BLOCKERS", reflection: "REFLECTION" },
  /** 領域の見出しの右の件数（`6 left`・`3 open`）。 */
  left: (n: number) => `${n} left`,
  open: (n: number) => `${n} open`,
};

/**
 * エージェントの識別子（agent_client の `name`）の表示名（画面設計書 3章「各領域の表示」の
 * 「◆ Claude Code」）。知っている識別子は製品の名前に、ほかは識別子のまま出す。
 */
const AGENT_NAMES: Record<string, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  cli: "CLI",
  unknown: "Agent",
};

export function agentLabel(name: string): string {
  return AGENT_NAMES[name] ?? name;
}

/** 保存に失敗した原因の種類（`SaveStatus.error` の `kind` から決める）。 */
export type SaveCause = "storage" | "not_ready" | "internal";


/** 起動時の画面の文言（画面設計書 10.1）。 */
export interface StartupMessages {
  opening: string;
  /** 開くのに時間がかかっているとき（ロックを待っているとき）。 */
  openingWait: string;
  notFoundTitle: string;
  searched: string;
  configuredIn: (path: string) => string;
  noFolder: string;
  maybeSyncing: string;
  /** 見つからないときの4つの案内（アーキテクチャ設計書 6.3）。 */
  waitTitle: string;
  waitBody: string;
  retry: string;
  chooseTitle: string;
  chooseBody: string;
  chooseLabel: string;
  choose: string;
  restoreTitle: string;
  restoreBody: string;
  /** 壊れていて開けないときの、バックアップから戻す案内。 */
  restoreDamagedBody: string;
  noBackups: string;
  backupsFailed: string;
  backupsLabel: string;
  restore: string;
  createTitle: string;
  createBody: string;
  create: string;
  createConfirm: string;
  createYes: string;
  createNo: string;
  working: string;
  /** 開けなかったときの見出しと説明。 */
  failedTitle: Record<StartupFailure, string>;
  failedBody: Record<StartupFailure, string>;
  /** 開けなかったときに示す場所。 */
  databaseAt: string;
  backupAt: string;
  configAt: string;
  /** 旧版を入れ直す案内（マイグレーションの失敗。ビルド・配布手順書 5章）。 */
  reinstallTitle: string;
  /** 入れ直すコマンドの、版の番号を入れる所。 */
  versionPlaceholder: string;
  reinstallBody: (version: string) => string;
  /** 壊れていて開けないときの、旧版の案内。 */
  reinstallDamagedBody: (version: string) => string;
  /** 新しい版を入れる案内（新しい版で作られたデータ）。 */
  upgradeTitle: string;
  upgradeBody: (version: string) => string;
  contactTitle: string;
  contactBody: string;
  contactIssues: string;
  contactMail: string;
  detailTitle: string;
  /** 選んだバックアップ・保存先を使えなかったとき。 */
  rejected: Record<RejectReason, string>;
  restoreFailed: string;
  /** 起動時の操作を始める前に止まった（ロックが取れないなど。`not_ready`）。 */
  notAttempted: string;
  /** 戻すのに失敗し、前のデータベースを元の場所に戻せなかった（`put_back_failed`）。 */
  putBackFailed: (path: string) => string;
  chooseFailed: string;
  createFailed: string;
  retryFailed: string;
  acknowledgeFailed: string;
  /** バックアップの一覧の種類。 */
  backupDaily: (date: string) => string;
  backupPreMigration: (from: number, to: number) => string;
  backupPreRestore: string;
  restoredTitle: string;
  restoredBody: string;
  restoredSetAside: (path: string) => string;
  continue: string;
}

export interface Messages {
  labels: typeof labels;
  /** 操作の名前にキーを添える（読み上げ用）。 */
  withKey: (label: string, key: string) => string;
  sidebar: string;
  boardMode: string;
  noteMode: string;
  modeSwitch: string;
  prevDay: string;
  nextDay: string;
  backToday: string;
  searchLabel: string;
  unseenChanges: (n: number) => string;
  leftToday: (n: number) => string;
  /** 凪メーターの読み上げ用のまとめ（画面設計書 3章「凪メーター」）。 */
  meterSummary: (left: number, todos: number, blockers: number, done: number, total: number, carried: number) => string;
  saving: string;
  saveFailed: string;
  saveCause: Record<SaveCause, string>;
  saveKeep: string;
  retry: string;
  keys: string;
  /** TODO が空のとき（画面設計書 3章「空の状態」）。`today` は今日を見ているか。 */
  emptyTodo: (today: boolean) => string;
  /** 取りやめた TODO の行末の理由（画面設計書 3章）。 */
  closeReason: Record<CloseReason, string>;
  /** 持ち越し回数の印の読み上げ。 */
  carryTimes: (n: number) => string;
  /** 持ち越し先の印（「→ 11.03」）の読み上げ。選ぶとその日を開く。 */
  openCarriedTo: (date: string) => string;
  /** 完了・解消のチェック欄の読み上げ。 */
  checkTodo: string;
  checkBlocker: string;
  /** 解消メモの欄の空のときの案内。 */
  memoPlaceholder: string;
  agentItem: string;
  agentUnseen: string;
  /** 持ち越し済みの項目を変えようとしたとき（操作仕様書 2章）。`date` は `11.03 Tue`。 */
  carriedNotice: (date: string) => string;
  /** 取りやめた TODO を変えようとしたとき（操作仕様書 2章）。 */
  droppedNotice: string;
  /** 削除を取り消せなかったとき（データモデル設計書 3章 item）。 */
  restoreSourceChanged: string;
  restoreSourceCarried: (date: string) => string;
  /** 送り直しても成功しない失敗で、保存を捨てて読み込み直したとき。 */
  saveRejected: string;
  /** 振り返りのテンプレート（Mod+Shift+R）の行。 */
  reflectionTemplate: string[];
  reflectionLabel: string;
  /** ノートのエディタの読み上げ用の名前。 */
  noteLabel: string;
  /** 項目の「← NOTE」の読み上げ。選ぶと、送り元のノートの行へ移る。 */
  openNoteSource: string;
  /** 行送りで、送れる行が無かったとき（見出し・空の行・コードの囲みの中など。操作仕様書 2章）。 */
  sendNothing: string;
  /** 行送りで、どの行もすでに送信済みだったとき。 */
  sendAlreadySent: string;
  /** 行送りに失敗したとき（本文はふだんの自動保存で送る）。 */
  sendFailed: string;
  /** ノートの行の右クリックのメニュー（操作仕様書 5章）。 */
  lineMenu: { label: string; toTodo: string; toBlocker: string };
  /** 定期バックアップを作れなかったとき（`backup-failed`。画面設計書 3章「上部のバー」）。 */
  backupFailed: string;
  startup: StartupMessages;
}


const jaStartup: StartupMessages = {
  opening: "データを開いています",
  openingWait: "CLI・MCP がデータを使っているときは、終わるまで最大 5 秒待ちます。",
  notFoundTitle: "データベースが見つかりません",
  searched: "探した場所",
  configuredIn: (path) => `保存先は ${path} に記録されています。`,
  noFolder: "保存先のフォルダがありません。外付けのドライブを繋いでいないか、同期がまだ終わっていないことがあります。",
  maybeSyncing: "同期がまだ終わっていないか、ファイルが動かされたことがあります。空のデータベースは作らずに止めています。",
  waitTitle: "待ってからやり直す",
  waitBody: "同期が終わる・ドライブを繋ぐのを待ってから、もう一度開きます。",
  retry: "やり直す",
  chooseTitle: "保存先を選び直す",
  chooseBody: "データベース（yunagi.db）のある別のフォルダを保存先にします。中身を確かめてから記録し、データは移しません。",
  chooseLabel: "フォルダのパス",
  choose: "このフォルダを使う",
  restoreTitle: "バックアップから戻す",
  restoreBody: "保存先の backups にあるバックアップから、データベースを作り直します。",
  restoreDamagedBody: "保存先の backups にあるバックアップから戻します。今のデータベースは消さずに backups に残します。",
  noBackups: "使えるバックアップがありません。",
  backupsFailed: "バックアップの一覧を読めませんでした。",
  backupsLabel: "戻すバックアップ",
  restore: "選んだバックアップから戻す",
  createTitle: "新しく作る",
  createBody: "空のデータベースを作って始めます。後から元のデータが届いても、自動ではまとまりません。",
  create: "新しく作る",
  createConfirm: "空のデータで始めます。よろしいですか？",
  createYes: "作る",
  createNo: "やめる",
  working: "処理しています",
  failedTitle: {
    default_dir: "アプリのフォルダを使えません",
    lock_busy: "ほかの処理がデータを使っています",
    lock: "アプリのフォルダを使えません",
    config: "設定ファイルを読めません",
    already_exists: "新しく作ろうとした場所に、データがあります",
    migration_failed: "更新後の準備に失敗しました",
    migration_restore_failed: "更新後の準備に失敗し、元に戻せませんでした",
    unsupported_schema: "このデータは新しい版の Yunagi で作られています",
    corrupted: "データベースが壊れているため、開けませんでした",
    storage: "データベースを開けませんでした",
  },
  failedBody: {
    default_dir: "設定とロックのファイルを置くフォルダを作れないか、開けません。権限と空き容量を確かめてから、やり直せます。",
    lock_busy: "CLI・MCP の操作が終わるのを 5 秒待ちましたが、終わりませんでした。終わってから、やり直せます。",
    lock: "ロックのファイル（yunagi.lock）を開けません。権限を確かめてから、やり直せます。",
    config: "JSON として読めないか、保存先が絶対パスではありません。別の場所に空のデータを作らないよう、開くのを止めています。直してから、やり直せます。",
    already_exists: "上書きしないため、作るのを止めました。やり直すと、そこにあるデータを開きます。",
    migration_failed: "新しい版に合わせてデータの形を変える処理（マイグレーション）が途中で失敗したため、更新前の状態に戻しました。データは無事です。",
    migration_restore_failed: "データの形を変える処理が途中で失敗し、更新前の状態に戻すこともできませんでした。更新前のデータは下のバックアップに残っています。ファイルを動かさずに、問い合わせ先へ知らせてください。",
    unsupported_schema: "この版では読めないため、データには触れていません。データは無事です。",
    corrupted: "整合性の確認が通らないか、データベースとして読めません。データベースには触れていません。",
    storage: "読み書きに失敗しました。保存先のドライブと空き容量を確かめてから、やり直せます。",
  },
  databaseAt: "データベース",
  backupAt: "更新前のバックアップ",
  configAt: "設定ファイル",
  reinstallTitle: "前の版を入れ直す",
  versionPlaceholder: "<版>",
  reinstallBody: (version) =>
    `前に使っていた版を入れ直すと、そのまま使えます。ターミナルで次を実行します（<版> には前に使っていた版の番号を入れます。今の版は ${version}）。`,
  reinstallDamagedBody: (version) =>
    `更新の直後なら、前に使っていた版ではそのまま開けることがあります。ターミナルで次を実行します（<版> には前に使っていた版の番号を入れます。今の版は ${version}）。`,
  upgradeTitle: "新しい版を入れる",
  upgradeBody: (version) =>
    `データを作った版か、それより新しい版で開けます。ターミナルで次を実行すると、最新の版が入ります（今の版は ${version}）。`,
  contactTitle: "問い合わせ先",
  contactBody: "解決しないときは、下の詳しい情報を添えて知らせてください。",
  contactIssues: "不具合報告（GitHub Issues）",
  contactMail: "GitHub のアカウントが無い場合",
  detailTitle: "詳しい情報（問い合わせのときに添える内容）",
  rejected: {
    invalid_path: "フォルダのパスは、/ や C:\\ から始まる形で入れます。",
    not_a_directory: "そのフォルダはありません。",
    no_database: "そのフォルダに yunagi.db がありません。",
    unreadable: "Yunagi のデータとして読めません。",
    corrupted: "壊れているため使えません。",
    not_yunagi: "Yunagi のデータベースではありません。",
    newer_schema: "新しい版の Yunagi で作られているため、この版では使えません。",
  },
  restoreFailed: "戻す途中で失敗したため、元の状態のままにしました。",
  notAttempted: "始める前に止まりました（ほかの処理がデータを使っているなど）。今の状態は上のとおりです。",
  putBackFailed: (path) =>
    `戻す途中で失敗し、それまでのデータベースを元の場所に戻すこともできませんでした。それまでのデータベースは ${path} に残っています。ファイルを動かさずに、問い合わせ先へ知らせてください。`,
  chooseFailed: "保存先を記録できませんでした。",
  createFailed: "データベースを作れませんでした。",
  retryFailed: "開き直せませんでした。",
  acknowledgeFailed: "進めませんでした。もう一度押せます。",
  backupDaily: (date) => `定期 ${date}`,
  backupPreMigration: (from, to) => `更新前（スキーマ ${from} → ${to}）`,
  backupPreRestore: "復元前",
  restoredTitle: "バックアップから戻しました",
  restoredBody: "バックアップを作った時点の内容になっています。",
  restoredSetAside: (path) => `それまでのデータベースは ${path} に残しています。`,
  continue: "続ける",
};

const enStartup: StartupMessages = {
  opening: "Opening your data",
  openingWait: "If the CLI or MCP is using the data, this waits up to 5 seconds for it to finish.",
  notFoundTitle: "Database not found",
  searched: "Looked in",
  configuredIn: (path) => `The data folder is set in ${path}.`,
  noFolder: "The data folder doesn't exist. An external drive may not be connected, or syncing may not have finished.",
  maybeSyncing: "Syncing may not have finished, or the file may have been moved. An empty database was not created.",
  waitTitle: "Wait and retry",
  waitBody: "Open again after syncing finishes or the drive is connected.",
  retry: "Retry",
  chooseTitle: "Choose the data folder again",
  chooseBody: "Use another folder that has a database (yunagi.db). It is checked before it is recorded, and no data is moved.",
  chooseLabel: "Folder path",
  choose: "Use this folder",
  restoreTitle: "Restore from a backup",
  restoreBody: "Recreate the database from a backup in the backups folder of the data folder.",
  restoreDamagedBody: "Restore from a backup in the backups folder. The current database is kept in backups, not deleted.",
  noBackups: "No backups are available.",
  backupsFailed: "Couldn't read the list of backups.",
  backupsLabel: "Backup to restore",
  restore: "Restore the selected backup",
  createTitle: "Create a new one",
  createBody: "Start with an empty database. If the original data arrives later, it won't be merged automatically.",
  create: "Create a new one",
  createConfirm: "Start with empty data?",
  createYes: "Create",
  createNo: "Cancel",
  working: "Working",
  failedTitle: {
    default_dir: "Can't use the app folder",
    lock_busy: "Something else is using the data",
    lock: "Can't use the app folder",
    config: "Can't read the settings file",
    already_exists: "Data already exists where a new one was to be created",
    migration_failed: "Preparing the data after the update failed",
    migration_restore_failed: "Preparing the data after the update failed and couldn't be undone",
    unsupported_schema: "This data was made by a newer version of Yunagi",
    corrupted: "The database is damaged and couldn't be opened",
    storage: "Couldn't open the database",
  },
  failedBody: {
    default_dir: "The folder for the settings and lock files can't be created or opened. You can retry after checking permissions and free space.",
    lock_busy: "Waited 5 seconds for the CLI or MCP to finish, but it didn't. You can retry once it finishes.",
    lock: "The lock file (yunagi.lock) can't be opened. You can retry after checking permissions.",
    config: "It isn't valid JSON, or the data folder isn't an absolute path. Opening stopped so that empty data isn't created somewhere else. You can retry after fixing it.",
    already_exists: "Creating stopped so nothing is overwritten. Retrying opens the data that is there.",
    migration_failed: "Updating the data to the new version (migration) failed partway, so the data was put back as it was before the update. Your data is safe.",
    migration_restore_failed: "Updating the data failed partway, and putting it back also failed. The data from before the update is kept in the backup below. Please leave the files as they are and contact support.",
    unsupported_schema: "This version can't read it, so the data was not touched. Your data is safe.",
    corrupted: "It failed the integrity check or can't be read as a database. The database was not touched.",
    storage: "Reading or writing failed. You can retry after checking the drive of the data folder and free space.",
  },
  databaseAt: "Database",
  backupAt: "Backup from before the update",
  configAt: "Settings file",
  reinstallTitle: "Reinstall the previous version",
  versionPlaceholder: "<version>",
  reinstallBody: (version) =>
    `Reinstalling the version you used before lets you keep using it as is. Run the following in a terminal (replace <version> with the version you used before; this one is ${version}).`,
  reinstallDamagedBody: (version) =>
    `Right after an update, the version you used before may still open it. Run the following in a terminal (replace <version> with the version you used before; this one is ${version}).`,
  upgradeTitle: "Install a newer version",
  upgradeBody: (version) =>
    `The version that made the data, or a newer one, can open it. Running the following in a terminal installs the latest version (this one is ${version}).`,
  contactTitle: "Contact",
  contactBody: "If this doesn't solve it, please report it with the details below.",
  contactIssues: "Bug reports (GitHub Issues)",
  contactMail: "Without a GitHub account",
  detailTitle: "Details (include these when you contact support)",
  rejected: {
    invalid_path: "Enter the folder path starting with / or C:\\.",
    not_a_directory: "That folder doesn't exist.",
    no_database: "That folder has no yunagi.db.",
    unreadable: "It can't be read as Yunagi data.",
    corrupted: "It is damaged and can't be used.",
    not_yunagi: "It isn't a Yunagi database.",
    newer_schema: "It was made by a newer version of Yunagi and can't be used by this one.",
  },
  restoreFailed: "Restoring failed partway, so everything was left as it was.",
  notAttempted: "It stopped before starting (for example, something else is using the data). The current state is shown above.",
  putBackFailed: (path) =>
    `Restoring failed partway, and the previous database couldn't be moved back either. It is kept at ${path}. Please leave the files as they are and contact support.`,
  chooseFailed: "Couldn't record the data folder.",
  createFailed: "Couldn't create the database.",
  retryFailed: "Couldn't open it again.",
  acknowledgeFailed: "Couldn't continue. You can press it again.",
  backupDaily: (date) => `Daily ${date}`,
  backupPreMigration: (from, to) => `Before update (schema ${from} → ${to})`,
  backupPreRestore: "Before restore",
  restoredTitle: "Restored from the backup",
  restoredBody: "The data is now as it was when the backup was made.",
  restoredSetAside: (path) => `The previous database is kept at ${path}.`,
  continue: "Continue",
};

const ja: Messages = {
  labels,
  withKey: (label, key) => `${label}（${key}）`,
  sidebar: "サイドバー",
  boardMode: "整理モード",
  noteMode: "ノートモード",
  modeSwitch: "整理モードとノートモードの切り替え",
  prevDay: "前日へ移動",
  nextDay: "翌日へ移動",
  backToday: "今日へ戻る",
  searchLabel: "検索・日付へ移動",
  unseenChanges: (n) => `エージェントによる未確認の変更 ${n} 件`,
  leftToday: (n) => `今日の残り ${n} 件`,
  meterSummary: (left, todos, blockers, done, total, carried) =>
    `残り ${left} 件（TODO ${todos}・障害 ${blockers}）、完了 ${done}/${total}、持ち越し ${carried}`,
  saving: "保存中",
  saveFailed: "保存できていません",
  saveCause: {
    storage: "保存先に書き込めません。",
    not_ready: "データベースを開けていません。",
    internal: "保存の処理が途中で止まりました。",
  },
  saveKeep: "入力した内容はこの画面に保持し、自動で再試行しています。",
  retry: "再試行",
  keys: "主なキー",
  emptyTodo: (today) => (today ? "今日やることを書く（Enter）" : "やることを書く（Enter）"),
  closeReason: { dropped: "やめた", split: "分割した", returned_external: "外部へ戻した" },
  carryTimes: (n) => `持ち越し ${n} 回`,
  openCarriedTo: (date) => `${date} へ持ち越し済み。その日を開く`,
  checkTodo: "完了",
  checkBlocker: "解消",
  memoPlaceholder: "解消メモ",
  agentItem: "エージェントが作った項目",
  agentUnseen: "エージェントによる未確認の変更",
  carriedNotice: (date) => `${date} へ持ち越し済みです`,
  droppedNotice: "持ち越しの見直しで閉じた TODO は変更できません",
  restoreSourceChanged: "持ち越し元の項目が変わったため、戻せません",
  restoreSourceCarried: (date) => `持ち越し元の項目が ${date} へ持ち越し済みのため、戻せません`,
  saveRejected: "変更を保存できなかったため、表示を読み込み直しました",
  reflectionTemplate: ["## 良かったこと", "- ", "", "## 詰まったこと", "- ", "", "## 明日の一手", "- "],
  reflectionLabel: "振り返り",
  noteLabel: "ノート",
  openNoteSource: "ノートの送り元の行へ移る",
  sendNothing: "送れる行がありません",
  sendAlreadySent: "送信済みの行です",
  sendFailed: "行を送れませんでした",
  lineMenu: { label: "行の操作", toTodo: "TODO へ送る", toBlocker: "障害へ送る" },
  backupFailed: "今日のバックアップを作れませんでした。自動で作り直します",
  startup: jaStartup,
};

const en: Messages = {
  labels,
  withKey: (label, key) => `${label} (${key})`,
  sidebar: "Sidebar",
  boardMode: "Board mode",
  noteMode: "Note mode",
  modeSwitch: "Switch between Board mode and Note mode",
  prevDay: "Previous day",
  nextDay: "Next day",
  backToday: "Back to today",
  searchLabel: "Search or jump to a date",
  unseenChanges: (n) => `${n} unseen changes by agents`,
  leftToday: (n) => `${n} left today`,
  meterSummary: (left, todos, blockers, done, total, carried) =>
    `${left} left (TODOs ${todos}, blockers ${blockers}), ${done} of ${total} done, ${carried} carried over`,
  saving: "Saving",
  saveFailed: "Not saved",
  saveCause: {
    storage: "Can't write to the save folder.",
    not_ready: "The database isn't open.",
    internal: "Saving stopped partway.",
  },
  saveKeep: "Your input is kept on this screen and saving is retried automatically.",
  retry: "Retry",
  keys: "Main keys",
  emptyTodo: (today) => (today ? "Write what to do today (Enter)" : "Write what to do (Enter)"),
  closeReason: { dropped: "Dropped", split: "Split", returned_external: "Returned" },
  carryTimes: (n) => `Carried over ${n} times`,
  openCarriedTo: (date) => `Carried over to ${date}. Open that day`,
  checkTodo: "Done",
  checkBlocker: "Resolved",
  memoPlaceholder: "Resolution note",
  agentItem: "Added by an agent",
  agentUnseen: "Unseen change by an agent",
  carriedNotice: (date) => `Carried over to ${date}`,
  droppedNotice: "This TODO was closed in the carry-over review and can't be changed",
  restoreSourceChanged: "Can't restore: the item it was carried over from has changed",
  restoreSourceCarried: (date) => `Can't restore: the item it was carried over from is carried over to ${date}`,
  saveRejected: "Couldn't save the change, so the board was reloaded",
  reflectionTemplate: ["## Went well", "- ", "", "## Got stuck", "- ", "", "## Next step for tomorrow", "- "],
  reflectionLabel: "Reflection",
  noteLabel: "Note",
  openNoteSource: "Go to the line in the note it was sent from",
  sendNothing: "No lines to send",
  sendAlreadySent: "Already sent",
  sendFailed: "Couldn't send the lines",
  lineMenu: { label: "Line actions", toTodo: "Send to TODO", toBlocker: "Send to Blockers" },
  backupFailed: "Today's backup couldn't be made. It will be retried automatically",
  startup: enStartup,
};

export const MESSAGES: Record<Lang, Messages> = { ja, en };

/** いまの言語の文言（言語を変えると、これを読んでいる表示が変わる）。 */
export function t(): Messages {
  return MESSAGES[prefs.lang];
}
