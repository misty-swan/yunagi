// 上部のバーに短時間だけ出す知らせ（画面設計書 3章「上部のバー」）。持ち越し済みの項目を変えようと
// したとき（操作仕様書 2章）、削除を戻せなかったときなど。見た目は TopBar が持つ。

/** 知らせを出しておく時間。 */
export const NOTICE_MS = 4000;
/** 長い知らせ（定期バックアップの失敗など、1日に1回だけ届くもの）を出しておく時間。 */
export const LONG_NOTICE_MS = 10000;

export class Notice {
  /** いま出している文。無ければ `null`。 */
  text = $state<string | null>(null);
  #timer: ReturnType<typeof setTimeout> | undefined;

  /** `text` を `ms`（既定 NOTICE_MS）の間だけ出す（前の知らせは置き換える）。 */
  show(text: string, ms = NOTICE_MS) {
    clearTimeout(this.#timer);
    this.text = text;
    this.#timer = setTimeout(() => (this.text = null), ms);
  }

  clear() {
    clearTimeout(this.#timer);
    this.text = null;
  }
}
