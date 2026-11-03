// 画面の文言（デザインガイド 7章）。
//
// - 日本語と英語は同じ情報量にそろえ、文言キー（このファイルの `Messages` の名前）で管理する。
// - ラベル（ナビ、凪メーター、キーの案内、件数、日付の表示など）は、日本語の画面でも英語で書く
//   （`labels`。日英で共通）。文（状態の説明、操作ボタン、エラーなど）は各言語で書く。
// - 短く、命令形を避け、状態を淡々と伝える。感嘆符・絵文字・励ましの言葉は使わない。

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
};

/** 保存に失敗した原因の種類（`SaveStatus.error` の `kind` から決める）。 */
export type SaveCause = "storage" | "not_ready" | "internal";

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
}

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
};

export const MESSAGES: Record<Lang, Messages> = { ja, en };

/** いまの言語の文言（言語を変えると、これを読んでいる表示が変わる）。 */
export function t(): Messages {
  return MESSAGES[prefs.lang];
}
