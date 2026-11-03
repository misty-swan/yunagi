// 画面の文言（デザインガイド 7章）。
//
// - 日本語と英語は同じ情報量にそろえ、文言キー（このファイルの `Messages` の名前）で管理する。
// - ラベル（ナビ、凪メーター、キーの案内、件数、日付の表示など）は、日本語の画面でも英語で書く
//   （`labels`。日英で共通）。文（状態の説明、操作ボタン、エラーなど）は各言語で書く。
// - 短く、命令形を避け、状態を淡々と伝える。感嘆符・絵文字・励ましの言葉は使わない。

import type { CloseReason } from "./bindings";
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
};

export const MESSAGES: Record<Lang, Messages> = { ja, en };

/** いまの言語の文言（言語を変えると、これを読んでいる表示が変わる）。 */
export function t(): Messages {
  return MESSAGES[prefs.lang];
}
