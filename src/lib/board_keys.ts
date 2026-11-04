// 整理モードの一覧のキー（操作仕様書 2章「標準ショートカット」の「整理」・「振り返り」と、
// 「取り消し・やり直し」）。割当を変えられる操作は keymap.ts（設定で変えた割当を当てたもの）、Enter と
// 取り消し・やり直し（Mod+Z・Mod+Shift+Z・Ctrl+Y）は変えられない。
//
// - IME の変換中のキーはショートカットとして扱わない（操作仕様書 3章）。変換を確定した直後の Enter も
//   扱わない（WebKit 系では compositionend の後に Enter の keydown が届く）が、それは画面の部品が
//   compositionend を見て判定する。

import { idsOf, type BoardAssigned } from "./keymap";
import { actionIn, composing } from "./keys";

/** 一覧の操作。 */
export type BoardAction = BoardAssigned | "enter" | "undo" | "redo";

type KeyLike = Pick<
  KeyboardEvent,
  "code" | "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey" | "isComposing" | "keyCode"
> & { getModifierState?: KeyboardEvent["getModifierState"] };

const BOARD_IDS = idsOf<BoardAssigned>("board");

/** キーの押下が、一覧の操作のどれに当たるか。当たらなければ `null`。 */
export function boardActionOf(e: KeyLike, mac: boolean): BoardAction | null {
  if (composing(e)) return null;
  const assigned = actionIn(e, mac, BOARD_IDS);
  if (assigned !== null) return assigned;
  const mod = mac ? e.metaKey : e.ctrlKey;
  const other = mac ? e.ctrlKey : e.metaKey;
  if (other) return null;
  if (!mod) return !e.altKey && !e.shiftKey && e.key === "Enter" ? "enter" : null;
  if (e.altKey) return null;
  if (e.code === "KeyZ") return e.shiftKey ? "redo" : "undo";
  if (e.code === "KeyY" && !mac && !e.shiftKey) return "redo";
  return null;
}
