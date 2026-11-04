// ふりかえり（S-08）の文言（画面設計書 9章、デザインガイド 7章）。i18n.ts の `Messages.archive`。
//
// 見出しの英字ラベル（This week / Daily notes / GOOD / STUCK / Open など）は、日英で共通
// （`labels.archive`）。ここには、読み上げの名前と状態の文を、各言語で書く。

export interface ArchiveMessages {
  /** ふりかえりの画面の名前（読み上げ）。 */
  label: string;
  /** 週のカードの一覧の名前（読み上げ）。 */
  weekLabel: string;
  /** Daily notes の一覧の名前（読み上げ）。 */
  dailyLabel: string;
  /** 選んだ日のプレビューの名前（読み上げ）。 */
  previewLabel: string;
  /** 週のカードの読み上げ：日付、完了数／TODO 数、今日か。 */
  card: (date: string, done: number, todos: number, today: boolean, future: boolean) => string;
  /** 記録のある日がまだ無いとき（Daily notes）。 */
  noDays: string;
  /** 選んだ日に何も無いとき（プレビュー）。 */
  noRecord: string;
  /** 振り返りを書いていない日（プレビュー）。 */
  noReflection: string;
  /** ノートを書いていない日（プレビュー）。 */
  noNote: string;
  /** 振り返りの GOOD / STUCK の行が無いとき。 */
  noLine: string;
  /** 「Open ↵」の読み上げ。 */
  openDay: (date: string) => string;
  /** 読み込みに失敗したとき（読み直している）。 */
  loadFailed: string;
}

export const ARCHIVE_MESSAGES: { ja: ArchiveMessages; en: ArchiveMessages } = {
  ja: {
    label: "ふりかえり",
    weekLabel: "週の各日",
    dailyLabel: "記録のある日",
    previewLabel: "選んだ日の内容",
    card: (date, done, todos, today, future) =>
      `${date}${today ? "（今日）" : ""}、${future ? "まだ先の日" : `完了 ${done}／TODO ${todos}`}`,
    noDays: "記録のある日はまだありません",
    noRecord: "この日の記録はありません",
    noReflection: "振り返りはありません",
    noNote: "ノートはありません",
    noLine: "—",
    openDay: (date) => `${date} のボードを開く`,
    loadFailed: "ふりかえりを読めませんでした。読み直しています。",
  },
  en: {
    label: "Archive",
    weekLabel: "Days of the week",
    dailyLabel: "Days with records",
    previewLabel: "Selected day",
    card: (date, done, todos, today, future) =>
      `${date}${today ? " (today)" : ""}, ${future ? "a future day" : `${done} of ${todos} TODOs done`}`,
    noDays: "No days with records yet",
    noRecord: "No records on this day",
    noReflection: "No reflection",
    noNote: "No note",
    noLine: "—",
    openDay: (date) => `Open the board for ${date}`,
    loadFailed: "Couldn't read the archive. Retrying.",
  },
};
