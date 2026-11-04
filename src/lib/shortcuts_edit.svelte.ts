// 設定（S-05）の「ショートカット割当」の操作：割り当てるキーを取り込んで、検査して、保存する
// （操作仕様書 1章。割当の決まりは keymap.ts）。
//
// - 「変更」を押すと取り込み中になり、次に押したキーを割当にする。取り込み中のキーは、ほかの画面の
//   操作にしない（呼ぶ側が止める）。Esc で取り消す。修飾キーだけの押下は待ち続ける。
// - 割り当てられないキーと、ほかの操作が使っているキーは、何も変えずに理由を出す（入れ替えも、ほかの
//   操作の割当の解除もしない）。
// - 初期割当と同じキーにしたときは、割当を持たない（keymap.ts `withBinding`）。

import { bindingFromEvent, conflictOf, problemOf, withBinding, type ActionId, type BindingProblem } from "./keymap";
import { composing, currentKeymap } from "./keys";
import { prefs } from "./prefs.svelte";
import type { SettingsStore } from "./settings.svelte";

/** 割当を変えられなかった理由。 */
export type EditMessage = { id: ActionId; problem: BindingProblem | "unsupported" } | { id: ActionId; conflict: ActionId };

export class ShortcutEditor {
  /** キーを取り込み中の操作。 */
  capturing = $state<ActionId | null>(null);
  /** 最後に割り当てられなかった理由（取り込みを始めるか、割り当てられると消す）。 */
  message = $state<EditMessage | null>(null);

  constructor(
    private store: SettingsStore,
    private mac: boolean,
  ) {}

  start(id: ActionId) {
    this.message = null;
    this.capturing = id;
  }

  cancel() {
    this.capturing = null;
  }

  /**
   * 取り込み中のキーの押下を受ける。取り込み中なら `true`（呼ぶ側は、ほかの画面へ渡さず、既定の動きも
   * 止める）。
   */
  onKey(e: KeyboardEvent): boolean {
    const id = this.capturing;
    if (id === null) return false;
    // IME の変換中は、変換のキーなので待つ
    if (composing(e)) return true;
    if (e.key === "Escape" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey) {
      this.cancel();
      return true;
    }
    // 修飾キーだけの押下は、組み合わせの途中なので待つ
    if (["Control", "Shift", "Alt", "Meta", "AltGraph", "OS"].includes(e.key)) return true;
    const b = bindingFromEvent(e, this.mac);
    if (b === null) {
      this.message = { id, problem: "unsupported" };
      this.capturing = null;
      return true;
    }
    const problem = problemOf(b, prefs.vimMode, this.mac, id);
    if (problem !== null) {
      this.message = { id, problem };
      this.capturing = null;
      return true;
    }
    const other = conflictOf(id, b, currentKeymap(this.mac));
    if (other !== null) {
      this.message = { id, conflict: other };
      this.capturing = null;
      return true;
    }
    this.message = null;
    this.capturing = null;
    void this.store.save({ shortcuts: withBinding(prefs.shortcuts, id, b, prefs.vimMode, this.mac) });
    return true;
  }

  /** 操作の割当を初期割当に戻す。 */
  reset(id: ActionId) {
    this.message = null;
    this.capturing = null;
    void this.store.save({ shortcuts: withBinding(prefs.shortcuts, id, null, prefs.vimMode, this.mac) });
  }

  /** すべての操作の割当を初期割当に戻す。 */
  resetAll() {
    this.message = null;
    this.capturing = null;
    void this.store.save({ shortcuts: {} });
  }
}
