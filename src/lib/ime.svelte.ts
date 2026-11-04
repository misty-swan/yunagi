// Vim の挿入モードの出入りに合わせた IME（日本語入力）の切替（操作仕様書 4.4）。
//
// 切替そのもの（OS の入力ソース・IMM・fcitx5 / IBus）はコア側（src-tauri/src/ime.rs）にあり、ここは
// 「挿入モードを抜けた」「挿入モードに入った」を知らせる口と、切替ができる環境かの問い合わせを持つ。
// 入力状態を覚えるのもコア側。
//
// - 呼ぶのは、Vimモードが有効なときだけ。窓が前面にないときも呼ばない（日が替わって挿入モードが終わった
//   など、利用者が触っていないときに、別のアプリで使っている入力状態を変えないため）。
// - 切替は待たない。失敗しても画面の操作は止めない（ノーマルモード中の扱いは画面側が保証する。4.4 の 2）。

import type { ImeStatus } from "./bindings";
import { core } from "./core";
import { prefs } from "./prefs.svelte";

/** 画面が触ってよい状態か：Vimモードが有効で、窓が前面にある。 */
function active(): boolean {
  return prefs.vimMode && (typeof document === "undefined" || document.hasFocus());
}

/** 挿入モードを抜けた（ノーマルモードに戻った）：IME を英数入力に切り替える（4.4 の 1）。 */
export function imeLeaveInsert(): void {
  if (!active()) return;
  void core.imeLeaveInsert().catch(() => {});
}

/**
 * 挿入モードに入った：直前に抜けた時点の入力状態に戻す。設定 `vim_ime_restore` が無効なら戻さない（4.4 の 3）。
 * 無効のときも入ったことは知らせる（コア側が、覚えていた入力状態を捨てるため）。
 */
export function imeEnterInsert(): void {
  if (!active()) return;
  void core.imeEnterInsert(prefs.vimImeRestore).catch(() => {});
}

/** この環境で IME の切替ができるか（設定画面の制約の表示。4.4 の 4）。 */
class ImeSupport {
  /** 問い合わせの結果（まだ、または問い合わせに失敗したときは `null`）。 */
  status = $state<ImeStatus | null>(null);

  /**
   * 問い合わせる（設定画面を開くたびに。利用者が後から fcitx5 や IBus を起動することがあるため）。
   * 失敗したら、切替ができないとは決めつけず、制約を表示しない。
   */
  async refresh(): Promise<void> {
    try {
      this.status = await core.imeStatus();
    } catch {
      this.status = null;
    }
  }

  /** 切替ができないと分かっているか。 */
  get unavailable(): boolean {
    return this.status?.available === false;
  }
}

export const imeSupport = new ImeSupport();
