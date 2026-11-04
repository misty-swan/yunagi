// OS 全体のショートカット（Yunagi を前面に呼び出す）の登録（操作仕様書 1章「割当の変更」、アーキテクチャ設計書 7章）。
//
// 登録そのもの（OS への登録・窓を前面に出す動き）はコア側（src-tauri/src/global_shortcut.rs）にあり、
// ここは「いまの割当を OS に登録し直してもらう」口と、登録の状態（設定に理由を出すため）を持つ。
//
// - 割当は、設定の `shortcuts` から求めたいまの割当（keymap.ts の `bringToFront`）。保存の呼び出し元ではなく、
//   割当の値そのものを見て、変わったときに登録し直す（`sync`）。保存に失敗して値が元に戻ったときも、
//   OS の登録が元のキーに戻る。データベースを開き直して設定を読み込み直したときも同じ。
// - 割当が使えない状態（同じキーをほかの操作が使っている）のときは `null` を渡して、登録を外す。
// - 登録できなくても、設定の保存は断らない（ほかのアプリが閉じれば使えるようになるため）。理由は
//   `status` に持ち、設定の「ショートカット割当」に出す。
// - 登録の呼び出しは順に 1 つずつ行う（続けて変えても、届く順が入れ替わらない）。

import type { GlobalShortcutStatus } from "./bindings";
import { core } from "./core";
import { format } from "./keymap";
import { currentKeymap } from "./keys";

class GlobalShortcutRegistry {
  /** 最後の登録の結果。まだ登録していない間と、呼び出しに失敗したときは `null`。 */
  status = $state<GlobalShortcutStatus | null>(null);
  /** 最後に OS へ頼んだ割当（`undefined` はまだ頼んでいない）。 */
  #requested: string | null | undefined = undefined;
  #queue: Promise<void> = Promise.resolve();

  /**
   * いまの割当を OS に登録する。前に頼んだ割当と同じなら何もしない。
   * 呼び出しに失敗したときは、登録できたか分からないので、次に呼ばれたとき同じ割当でも頼み直す。
   */
  sync(mac: boolean): Promise<void> {
    const b = currentKeymap(mac).get("bringToFront")?.binding ?? null;
    const binding = b === null ? null : format(b);
    if (binding === this.#requested) return this.#queue;
    this.#requested = binding;
    this.#queue = this.#queue.then(async () => {
      try {
        this.status = await core.setGlobalShortcut(binding);
      } catch {
        this.status = null;
        this.#requested = undefined;
      }
    });
    return this.#queue;
  }

  /** 登録を外した状態に戻す（データベースを開き直す前など。次の `sync` で必ず頼み直す）。 */
  forget() {
    this.#requested = undefined;
    this.status = null;
  }
}

export const globalShortcut = new GlobalShortcutRegistry();
