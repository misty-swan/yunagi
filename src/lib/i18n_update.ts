// 自動更新の文言：「New version」の通知と、更新内容・「更新して再起動」の表示（画面設計書 3章）。
// i18n.ts の `Messages.update`。見出しの英字ラベル「New version」は日英で共通（`label`）。

export interface UpdateMessages {
  /** サイドバー下端・上部のバーの通知のラベル（日英で共通）。 */
  label: string;
  /** 通知を選んで開く表示の名前（読み上げ）。 */
  dialog: string;
  /** 新しい版の見出し。 */
  heading: (version: string) => string;
  /** 更新内容が無いとき。 */
  noNotes: string;
  /** 「更新して再起動」（ボタンとパレットのコマンド）。 */
  install: string;
  close: string;
  /** ダウンロード中。割合が分からないときは `null`。 */
  downloading: (percent: number | null) => string;
  restarting: string;
  /** ダウンロードか署名の検証に失敗した（何も入れ替えていない）。 */
  failed: string;
  /** 保存できていないので、更新を始めない。 */
  saveFailing: string;
  /** 更新前の案内：入力中の内容は保存してから再起動する。 */
  hint: string;
}

const ja: UpdateMessages = {
  label: "New version",
  dialog: "新しい版",
  heading: (version) => `新しい版 ${version}`,
  noNotes: "更新内容は届いていません。",
  install: "更新して再起動",
  close: "閉じる",
  downloading: (percent) => (percent === null ? "ダウンロードしています" : `ダウンロードしています ${percent}%`),
  restarting: "再起動します",
  failed: "更新できませんでした。何も入れ替えていません。もう一度試せます。",
  saveFailing: "保存できていないので、更新を始めません。保存できてからやり直してください。",
  hint: "入力中の内容は保存してから再起動します。",
};

const en: UpdateMessages = {
  label: "New version",
  dialog: "New version",
  heading: (version) => `Version ${version} is available`,
  noNotes: "No release notes were provided.",
  install: "Update and restart",
  close: "Close",
  downloading: (percent) => (percent === null ? "Downloading" : `Downloading ${percent}%`),
  restarting: "Restarting",
  failed: "Couldn't update. Nothing was replaced. You can try again.",
  saveFailing: "Not saved yet, so the update hasn't started. Try again once it saves.",
  hint: "Your input is saved before it restarts.",
};

export const UPDATE_MESSAGES = { ja, en };
