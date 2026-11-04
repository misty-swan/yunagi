// 画面の骨組みの状態：モード（整理／ノート）とサイドバーの開閉（画面設計書 3章「サイドバー」・
// 「レイアウト」、4章）。見た目は持たない。
//
// サイドバーを出すかは、次の順で決める。
// - ノートモード：入ると閉じる。ノートモードの間に Mod+\ で開け閉めでき、整理モードに戻ると
//   その開け閉めは捨てて、整理モードでの状態に戻す。
// - ウィンドウ幅 900px 未満：閉じる。Mod+\ で明示的に開いたときだけ出す。幅が 900px 以上に戻ると、
//   その開け閉めは捨てる（次に狭くなったときはまた閉じる）。
// - それ以外：利用者が Mod+\ で決めた状態（既定は開く）。

/** 画面のモード。 */
export type Mode = "board" | "note";

/** これより狭いウィンドウでは、サイドバーを閉じ、整理モードを縦積みにする（画面設計書 3章）。 */
export const NARROW_PX = 900;

export class Shell {
  mode = $state<Mode>("board");
  /** ウィンドウの幅（px）。 */
  width = $state(Infinity);
  /** 広いウィンドウの整理モードで、サイドバーを開いているか（利用者が Mod+\ で決める）。 */
  #wideOpen = $state(true);
  /** 狭いウィンドウで、明示的に開いたか。 */
  #narrowOpen = $state(false);
  /** ノートモードの間に開け閉めした状態（整理モードに戻ると捨てる）。 */
  #noteOpen = $state(false);
  /**
   * 広いウィンドウの整理モードで、利用者が開け閉めしたときの知らせ（設定 `sidebar` に保存する。
   * ノートモード・狭いウィンドウでの一時的な開け閉めは知らせない）。
   */
  onWideOpenChange: ((open: boolean) => void) | null = null;

  get narrow(): boolean {
    return this.width < NARROW_PX;
  }

  /** サイドバーを出すか。 */
  get sidebarOpen(): boolean {
    if (this.mode === "note") return this.#noteOpen;
    return this.narrow ? this.#narrowOpen : this.#wideOpen;
  }

  /** サイドバーを開く・閉じる（Mod+\）。 */
  toggleSidebar() {
    const open = !this.sidebarOpen;
    if (this.mode === "note") this.#noteOpen = open;
    else if (this.narrow) this.#narrowOpen = open;
    else {
      this.#wideOpen = open;
      this.onWideOpenChange?.(open);
    }
  }

  /** 保存してあった、広いウィンドウの整理モードでのサイドバーの開閉を当てる（起動のとき）。 */
  setWideOpen(open: boolean) {
    this.#wideOpen = open;
  }

  setMode(mode: Mode) {
    if (mode === this.mode) return;
    this.mode = mode;
    this.#noteOpen = false;
  }

  /** 整理モードとノートモードを切り替える（Mod+J）。 */
  toggleMode() {
    this.setMode(this.mode === "board" ? "note" : "board");
  }

  setWidth(width: number) {
    if (width >= NARROW_PX) this.#narrowOpen = false;
    this.width = width;
  }
}
