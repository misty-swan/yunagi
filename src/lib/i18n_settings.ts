// 設定画面（S-05）の文言（画面設計書 7章、デザインガイド 7章）。i18n.ts の `Messages.settings`。
//
// 区分の名前（General / Display / Operation / Data / Agents）は英字ラベルで、日英で共通
// （`labels.settings`）。設定の項目名と説明は、各言語で書く。

import type { CloudService, Layout, NoteFont, RejectReason, Theme } from "./bindings";
import type { ActionId, BindingProblem, Scope } from "./keymap";

export interface SettingsMessages {
  /** 設定の画面の名前（読み上げ）。 */
  label: string;
  /** 区分の一覧の名前（読み上げ）。 */
  sectionsLabel: string;
  close: string;
  /** 設定を保存できなかったとき。 */
  saveFailed: string;
  invalid: string;
  general: {
    dayBoundary: string;
    dayBoundaryNote: string;
    meterHours: string;
    meterStart: string;
    meterEnd: string;
    meterNote: string;
    language: string;
    languageOptions: Record<"ja" | "en", string>;
    update: string;
    updateNote: string;
  };
  display: {
    theme: string;
    themeOptions: Record<Theme, string>;
    fontSize: string;
    fontSizeValue: (px: number) => string;
    noteFont: string;
    noteFontOptions: Record<NoteFont, string>;
    layout: string;
    layoutOptions: Record<Layout, string>;
    layoutNote: string;
  };
  operation: {
    reviewThreshold: string;
    reviewThresholdNote: string;
    shortcuts: string;
    shortcutsNote: string;
    /** 操作の効く範囲ごとの見出し。 */
    scopes: Record<Scope, string>;
    /** 操作の名前。 */
    actions: Record<ActionId, string>;
    /** 割当のボタンの読み上げ（`action` の割当 `key` を変える）。 */
    changeKey: (action: string, key: string) => string;
    /** キーの取り込み中の表示。 */
    pressKey: string;
    reset: string;
    resetAll: string;
    /** 割当を変えられなかった理由。 */
    problems: Record<BindingProblem | "unsupported", string>;
    /** 同じキーをほかの操作が使っている。 */
    conflict: (action: string) => string;
    /** 変えた割当と重なって、初期割当のキーが使えない。 */
    lost: (action: string) => string;
    /** 設定の割当が使えないキーなので、初期割当にしている。 */
    ignored: string;
    vim: string;
    vimNote: string;
    /** Windows・Linux の Vim モードの割当の注意。 */
    vimAltNote: string;
    vimImeRestore: string;
    vimImeRestoreNote: string;
    /** Linux で IME の切替ができない環境の制約（操作仕様書 4.4 の 4）。 */
    imeUnavailable: string;
  };
  data: {
    locationTitle: string;
    location: string;
    unknownLocation: string;
    move: string;
    movePick: string;
    switchTo: string;
    switchPick: string;
    syncNote: string;
    cloudWarning: (service: string) => string;
    cloud: Record<CloudService, string>;
    moveConfirm: (dir: string) => string;
    moveYes: string;
    switchConfirm: (dir: string) => string;
    switchYes: string;
    no: string;
    working: string;
    moved: string;
    leftovers: (n: number) => string;
    moveFailed: string;
    switchFailed: string;
    /** 保存先の変更・選び直しを使えなかった理由（`rejected`）。 */
    rejected: Record<RejectReason, string>;
    backupsNote: string;
    backupsEmpty: string;
    backupsFailed: string;
    restoreButton: (label: string) => string;
    restoreConfirm: (label: string, time: string) => string;
    restoreYes: string;
    restoreFailed: string;
    exportTitle: string;
    exportFrom: string;
    exportTo: string;
    exportDir: string;
    exportNoDir: string;
    exportPick: string;
    exportPickTitle: string;
    exportRun: string;
    exported: (n: number, dir: string) => string;
    exportNothing: string;
    exportWriteFailed: string;
    exportPartial: (n: number) => string;
    exportReadFailed: string;
    /** 保存できていない入力があるので、選び直し・復元をしなかった。 */
    unsaved: string;
  };
  agents: {
    connected: string;
    permissions: string;
    localApi: string;
    history: string;
    openHistory: string;
    notYet: string;
  };
}

const ja: SettingsMessages = {
  label: "設定",
  sectionsLabel: "設定の区分",
  close: "閉じる",
  saveFailed: "設定を保存できませんでした。前の値に戻しました。",
  invalid: "この値は設定できません。前の値に戻しました。",
  general: {
    dayBoundary: "日付の切り替わり時刻",
    dayBoundaryNote: "この時刻より前は、前の日の続きとして扱います。変えた後に作るデータにだけ使い、いまある日付は付け直しません。",
    meterHours: "凪メーターの時間帯",
    meterStart: "始業",
    meterEnd: "終業",
    meterNote: "始業は終業より前の時刻にします。",
    language: "言語",
    languageOptions: { ja: "日本語", en: "English" },
    update: "更新の自動確認",
    updateNote: "無効にすると、起動時と定期の確認を行いません。",
  },
  display: {
    theme: "テーマ",
    themeOptions: { system: "OS に合わせる", light: "ライト", dark: "ダーク" },
    fontSize: "フォントサイズ",
    fontSizeValue: (px) => `${px}px`,
    noteFont: "ノートの書体",
    noteFontOptions: { body: "本文と同じ", mono: "等幅" },
    layout: "レイアウト",
    layoutOptions: { A: "A. 2段", B: "B. 3列", C: "C. 縦積み" },
    layoutNote: "ウィンドウの幅が 900px 未満の間は、設定によらず C で表示します。",
  },
  operation: {
    reviewThreshold: "持ち越し見直しの回数",
    reviewThresholdNote: "TODO の持ち越し回数がこの回数以上になると、持ち越しの画面に「分割する／やめる」を添え、回数の印の色を変えます。",
    shortcuts: "ショートカット割当",
    shortcutsNote:
      "「変更」を押してから、割り当てるキーを押します。Mod は macOS では ⌘、Windows・Linux では Ctrl です。取り消し・やり直し、コピー・貼り付けなどのキーは変えられません。",
    scopes: { common: "共通", board: "整理モード", note: "ノートモード" },
    actions: {
      toggleMode: "ノートモード ⇄ 整理モード",
      prevDay: "前日へ移動（ふりかえりでは前の週）",
      nextDay: "翌日へ移動（ふりかえりでは次の週）",
      today: "今日へ戻る",
      palette: "検索パレット",
      settings: "設定",
      sidebar: "サイドバーを開く・閉じる",
      archive: "ふりかえりを開く・閉じる",
      regionTodo: "TODO の領域へ移る",
      regionBlocker: "障害の領域へ移る",
      regionReflection: "振り返りの領域へ移る",
      toggle: "完了・解消の切替",
      moveUp: "項目を上へ移動",
      moveDown: "項目を下へ移動",
      delete: "項目の削除",
      template: "振り返りテンプレートの挿入",
      toTodo: "現在行を TODO へ送る",
      toBlocker: "現在行を障害へ送る",
      timeHeading: "時刻付き見出しの挿入",
    },
    changeKey: (action, key) => `${action}のキーを変える（いまは ${key === "" ? "なし" : key}）`,
    pressKey: "新しいキーを押してください（Esc で取り消し）",
    reset: "初期値に戻す",
    resetAll: "すべて初期値に戻す",
    problems: {
      no_modifier: "Mod か Alt と組み合わせたキーにしてください（文字の入力と区別できないため）。",
      mac_option_alone: "macOS の Option は文字を打つキーなので、⌘ と組み合わせてください。",
      edit_key: "このキーは文字の編集や移動に使うので割り当てられません（Mod と組み合わせられるのは Enter と Backspace、Alt だけで使えるのは上下の矢印だけです）。",
      alt_gr: "Windows・Linux では Ctrl と Alt を同時に使えません（AltGr で文字を打つ配列があるため）。",
      shifted_symbol: "記号のキーには Shift を付けられません（配列によって打つ文字が変わるため）。",
      mac_option_symbol: "記号のキーには Option を付けられません（配列によって打つ文字が変わるため）。",
      reserved: "このキーはほかの操作が使うので、割り当てられません。",
      unsupported: "このキーは割り当てられません。",
    },
    conflict: (action) => `「${action}」と同じキーです。先にそちらを変えてください。`,
    lost: (action) => `「${action}」に割り当てたキーと重なっているため、いまは使えません。`,
    ignored: "設定の割当が使えないキーなので、初期割当にしています。",
    vim: "Vim モード",
    vimNote:
      "ノートと振り返りのエディタ、整理モードの一覧を Vim のキーで操作します。Mod を使う標準のショートカットは、そのまま使えます。",
    vimAltNote:
      "Windows・Linux では Ctrl+[ が Vim の Esc と重なるため、前日・翌日の移動（ふりかえりでは週の移動）の初期割当を Alt+[ / Alt+] にします。",
    vimImeRestore: "挿入モードで入力状態を戻す",
    vimImeRestoreNote: "挿入モードに入ったとき、直前に挿入モードを抜けた時点の日本語入力の状態に戻します。",
    imeUnavailable:
      "この環境では日本語入力（IME）を自動で切り替えられません（fcitx5 か IBus が動いていません）。ノーマルモード中は、日本語入力がオンでもキーを Vim の操作として扱います。挿入モードを抜けたら、日本語入力を手で英数に切り替えてください。",
  },
  data: {
    locationTitle: "保存先",
    location: "いまの保存先",
    unknownLocation: "読み込めませんでした",
    move: "保存先を変える",
    movePick: "移す先のフォルダを選ぶ",
    switchTo: "既存のフォルダを選び直す",
    switchPick: "データベースのあるフォルダを選ぶ",
    syncNote:
      "クラウド同期フォルダ（iCloud Drive・Dropbox など）に置くのは、同時に1台だけで開く場合に限ります。複数の端末で使うときは、Markdown の書き出しかバックアップの同期を使います。macOS の「デスクトップと書類」の iCloud 同期や、Windows の OneDrive のフォルダのバックアップのように、書類などの既定のフォルダをそのまま同期するものは判定できません。",
    cloudWarning: (service) =>
      `${service} の同期フォルダにあるようです。データベースと WAL のファイルが別々に同期されると、壊れることがあります。`,
    cloud: {
      i_cloud_drive: "iCloud Drive",
      dropbox: "Dropbox",
      one_drive: "OneDrive",
      google_drive: "Google ドライブ",
    },
    moveConfirm: (dir) => `データベースとバックアップを ${dir} へ移します。`,
    moveYes: "移す",
    switchConfirm: (dir) =>
      `${dir} にあるデータに切り替えます。いまのデータは移さず、いまの保存先に残ります。画面を読み込み直します。`,
    switchYes: "切り替える",
    no: "やめる",
    working: "処理しています",
    moved: "保存先を変えました。",
    leftovers: (n) => `元の保存先に消せなかったものが ${n} 件あります。データは新しい保存先に移っています。`,
    moveFailed: "保存先を変えられませんでした。データは元のままです。",
    switchFailed: "保存先を選び直せませんでした。",
    rejected: {
      invalid_path: "フォルダのパスは、/ や C:\\ から始まる形で、. や .. を含まないものにします。",
      not_a_directory: "そのフォルダはありません。",
      no_database: "そのフォルダか、いまの保存先に、データベース（yunagi.db）が見つかりません。",
      unreadable: "Yunagi のデータとして読めません。",
      corrupted: "壊れているため使えません。",
      not_yunagi: "Yunagi のデータベースではありません。",
      newer_schema: "新しい版の Yunagi で作られているため、この版では使えません。",
      same_directory: "いまの保存先と同じフォルダです。",
      inside_backups: "いまの保存先の backups の中には移せません。",
      target_has_data: "そのフォルダには、すでにデータベースかバックアップがあります（上書きしません）。",
    },
    backupsNote: "定期は直近7日分、更新前と復元前はそれぞれ新しい3つを残します。世代数は変えられません。",
    backupsEmpty: "使えるバックアップがありません。",
    backupsFailed: "バックアップの一覧を読めませんでした。",
    restoreButton: (label) => `${label}から復元する`,
    restoreConfirm: (label, time) =>
      `${label}（${time}）に戻します。いまのデータは「復元前」のバックアップに残します。画面を読み込み直します。`,
    restoreYes: "戻す",
    restoreFailed: "復元できませんでした。データは元のままです。",
    exportTitle: "Markdown 書き出し",
    exportFrom: "開始日",
    exportTo: "終了日",
    exportDir: "書き出し先",
    exportNoDir: "フォルダを選んでいません",
    exportPick: "フォルダを選ぶ",
    exportPickTitle: "書き出し先のフォルダを選ぶ",
    exportRun: "書き出す",
    exported: (n, dir) => `${n} 件のファイルを ${dir} に書き出しました。`,
    exportNothing: "書き出すものがない期間です。ファイルは作っていません。",
    exportWriteFailed: "書き出し先に書き込めませんでした。フォルダがあるか、書き込めるか、空き容量があるかを確かめてください。",
    exportPartial: (n) => `${n} 件までは書き換え済みです。`,
    exportReadFailed: "データを読めなかったため、書き出せませんでした。",
    unsaved: "保存できていない入力があるため、切り替えませんでした。保存できてから、もう一度行ってください。",
  },
  agents: {
    connected: "接続中のエージェント",
    permissions: "権限",
    localApi: "ローカル API",
    history: "変更履歴",
    openHistory: "変更履歴を開く",
    notYet: "まだ変えられません。",
  },
};

const en: SettingsMessages = {
  label: "Settings",
  sectionsLabel: "Settings sections",
  close: "Close",
  saveFailed: "Couldn't save the setting. It was set back to the previous value.",
  invalid: "That value can't be set. It was set back to the previous value.",
  general: {
    dayBoundary: "Day boundary",
    dayBoundaryNote: "Times before this belong to the previous day. It applies only to data made after the change; existing days keep their dates.",
    meterHours: "Nagi meter hours",
    meterStart: "Start",
    meterEnd: "End",
    meterNote: "The start must be earlier than the end.",
    language: "Language",
    languageOptions: { ja: "日本語", en: "English" },
    update: "Check for updates automatically",
    updateNote: "When off, Yunagi doesn't check at launch or periodically.",
  },
  display: {
    theme: "Theme",
    themeOptions: { system: "Match the OS", light: "Light", dark: "Dark" },
    fontSize: "Font size",
    fontSizeValue: (px) => `${px}px`,
    noteFont: "Note font",
    noteFontOptions: { body: "Same as body", mono: "Monospace" },
    layout: "Layout",
    layoutOptions: { A: "A. Two rows", B: "B. Three columns", C: "C. Stacked" },
    layoutNote: "While the window is narrower than 900px, it is shown as C regardless of this setting.",
  },
  operation: {
    reviewThreshold: "Carry-over review count",
    reviewThresholdNote: "Once a TODO has been carried over this many times, the carry-over screen adds Split / Drop, and the count mark changes color.",
    shortcuts: "Keyboard shortcuts",
    shortcutsNote:
      "Press Change, then press the key to assign. Mod is ⌘ on macOS and Ctrl on Windows and Linux. Undo, redo, copy, paste and similar keys can't be changed.",
    scopes: { common: "Everywhere", board: "Board mode", note: "Note mode" },
    actions: {
      toggleMode: "Switch Note ⇄ Board mode",
      prevDay: "Previous day (previous week in Archive)",
      nextDay: "Next day (next week in Archive)",
      today: "Back to today",
      palette: "Search palette",
      settings: "Settings",
      sidebar: "Open / close the sidebar",
      archive: "Open / close Archive",
      regionTodo: "Go to the TODO area",
      regionBlocker: "Go to the Blockers area",
      regionReflection: "Go to the Reflection area",
      toggle: "Toggle done / resolved",
      moveUp: "Move item up",
      moveDown: "Move item down",
      delete: "Delete item",
      template: "Insert reflection template",
      toTodo: "Send current line to TODO",
      toBlocker: "Send current line to Blockers",
      timeHeading: "Insert timestamp heading",
    },
    changeKey: (action, key) => `Change the key for ${action} (now ${key === "" ? "none" : key})`,
    pressKey: "Press the new key (Esc to cancel)",
    reset: "Reset",
    resetAll: "Reset all",
    problems: {
      no_modifier: "Use a key combined with Mod or Alt (otherwise it can't be told from typing).",
      mac_option_alone: "Option types characters on macOS. Combine the key with ⌘.",
      edit_key: "This key is used for editing and moving the caret, so it can't be assigned (with Mod only Enter and Backspace are allowed; with Alt alone only the up and down arrows).",
      alt_gr: "On Windows and Linux, Ctrl and Alt can't be used together (some layouts type characters with AltGr).",
      shifted_symbol: "A symbol key can't be combined with Shift (the character depends on the layout).",
      mac_option_symbol: "A symbol key can't be combined with Option (the character depends on the layout).",
      reserved: "This key is used by another operation and can't be assigned.",
      unsupported: "This key can't be assigned.",
    },
    conflict: (action) => `That key is already used by "${action}". Change that one first.`,
    lost: (action) => `Disabled for now: the key is assigned to "${action}".`,
    ignored: "The saved key can't be used, so the default is in effect.",
    vim: "Vim mode",
    vimNote:
      "Operate the Note and Reflection editors and the Board lists with Vim keys. The standard Mod shortcuts keep working.",
    vimAltNote:
      "On Windows and Linux, Ctrl+[ overlaps Vim's Esc, so the default for previous / next day (and week in Archive) becomes Alt+[ / Alt+].",
    vimImeRestore: "Restore input state in Insert mode",
    vimImeRestoreNote: "When entering Insert mode, return the Japanese input to the state it had when you last left Insert mode.",
    imeUnavailable:
      "Yunagi can't switch the Japanese input (IME) automatically here (neither fcitx5 nor IBus is running). In Normal mode, keys still work as Vim commands even when the IME is on. After leaving Insert mode, switch the IME to English by hand.",
  },
  data: {
    locationTitle: "Data folder",
    location: "Current folder",
    unknownLocation: "Couldn't be read",
    move: "Change the data folder",
    movePick: "Choose the folder to move to",
    switchTo: "Choose an existing folder",
    switchPick: "Choose the folder that has the database",
    syncNote:
      "Use a cloud sync folder (iCloud Drive, Dropbox, and so on) only when you open Yunagi on one device at a time. To use several devices, use Markdown export or sync the backups. Folders that sync documents wholesale, such as iCloud's Desktop & Documents on macOS or OneDrive's folder backup on Windows, can't be detected.",
    cloudWarning: (service) =>
      `This looks like a ${service} sync folder. If the database and its WAL file sync separately, they can be damaged.`,
    cloud: {
      i_cloud_drive: "iCloud Drive",
      dropbox: "Dropbox",
      one_drive: "OneDrive",
      google_drive: "Google Drive",
    },
    moveConfirm: (dir) => `Move the database and backups to ${dir}.`,
    moveYes: "Move",
    switchConfirm: (dir) =>
      `Switch to the data in ${dir}. The current data isn't moved and stays in the current folder. The screen reloads.`,
    switchYes: "Switch",
    no: "Cancel",
    working: "Working",
    moved: "The data folder was changed.",
    leftovers: (n) => `${n} items in the old folder couldn't be deleted. The data has moved to the new folder.`,
    moveFailed: "Couldn't change the data folder. The data is as it was.",
    switchFailed: "Couldn't switch the data folder.",
    rejected: {
      invalid_path: "The folder path must start with / or C:\\ and must not contain . or ..",
      not_a_directory: "That folder doesn't exist.",
      no_database: "No database (yunagi.db) was found in that folder or in the current folder.",
      unreadable: "It can't be read as Yunagi data.",
      corrupted: "It's damaged, so it can't be used.",
      not_yunagi: "It isn't a Yunagi database.",
      newer_schema: "It was made by a newer version of Yunagi, so this version can't use it.",
      same_directory: "That's the current folder.",
      inside_backups: "It can't be moved into the backups folder of the current folder.",
      target_has_data: "That folder already has a database or backups. They aren't overwritten.",
    },
    backupsNote: "Daily backups keep the latest 7, and before-update and before-restore backups keep the newest 3 each. The numbers can't be changed.",
    backupsEmpty: "No backups are available.",
    backupsFailed: "Couldn't read the list of backups.",
    restoreButton: (label) => `Restore ${label}`,
    restoreConfirm: (label, time) =>
      `Restore ${label} (${time}). The current data is kept as a before-restore backup. The screen reloads.`,
    restoreYes: "Restore",
    restoreFailed: "Couldn't restore. The data is as it was.",
    exportTitle: "Export to Markdown",
    exportFrom: "From",
    exportTo: "To",
    exportDir: "Export folder",
    exportNoDir: "No folder chosen",
    exportPick: "Choose a folder",
    exportPickTitle: "Choose the export folder",
    exportRun: "Export",
    exported: (n, dir) => `Exported ${n} files to ${dir}.`,
    exportNothing: "There is nothing to export in that period. No files were made.",
    exportWriteFailed: "Couldn't write to the export folder. Check that it exists, is writable, and has free space.",
    exportPartial: (n) => `${n} files had already been replaced.`,
    exportReadFailed: "Couldn't read the data, so nothing was exported.",
    unsaved: "Some input isn't saved yet, so nothing was switched. Try again once it is saved.",
  },
  agents: {
    connected: "Connected agents",
    permissions: "Permissions",
    localApi: "Local API",
    history: "Change history",
    openHistory: "Open change history",
    notYet: "Can't be changed yet.",
  },
};

export const SETTINGS_MESSAGES = { ja, en };
