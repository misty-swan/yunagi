// 上部のバーに短時間だけ出す知らせ（画面設計書 3章「上部のバー」）。持ち越し済みの項目を変えようと
// したとき（操作仕様書 2章）、削除を戻せなかったときなど。見た目は TopBar が持つ。

/** 知らせを出しておく時間。 */
export const NOTICE_MS = 4000;

export class Notice {
  /** いま出している文。無ければ `null`。 */
  text = $state<string | null>(null);
  #timer: ReturnType<typeof setTimeout> | undefined;

  /** `text` を NOTICE_MS の間だけ出す（前の知らせは置き換える）。 */
  show(text: string) {
    clearTimeout(this.#timer);
    this.text = text;
    this.#timer = setTimeout(() => (this.text = null), NOTICE_MS);
  }

  clear() {
    clearTimeout(this.#timer);
    this.text = null;
  }
}
