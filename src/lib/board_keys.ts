// 整理モードの一覧のキー（操作仕様書 2章「標準ショートカット」の「整理」・「振り返り」と、
// 「取り消し・やり直し」。初期割当。割当の変更は設定の課題で作る）。
//
// - IME の変換中のキーはショートカットとして扱わない（操作仕様書 3章）。変換を確定した直後の Enter も
//   扱わない（WebKit 系では compositionend の後に Enter の keydown が届く）が、それは画面の部品が
//   compositionend を見て判定する。
// - 文字のキー（Z・Y・R）はキーの位置（`code`）で、数字（1〜3）も位置で判定する（keys.ts と同じ）。

/** 一覧の操作。 */
export type BoardAction =
  | "enter"
  | "toggle"
  | "delete"
  | "moveUp"
  | "moveDown"
  | "undo"
  | "redo"
  | "regionTodo"
  | "regionBlocker"
  | "regionReflection"
  | "template";

type KeyLike = Pick<
  KeyboardEvent,
  "code" | "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey" | "isComposing" | "keyCode"
>;

const REGIONS: Partial<Record<string, BoardAction>> = {
  Digit1: "regionTodo",
  Digit2: "regionBlocker",
  Digit3: "regionReflection",
};

/** キーの押下が、一覧の操作のどれに当たるか。当たらなければ `null`。 */
export function boardActionOf(e: KeyLike, mac: boolean): BoardAction | null {
  if (e.isComposing || e.keyCode === 229) return null;
  const mod = mac ? e.metaKey : e.ctrlKey;
  const other = mac ? e.ctrlKey : e.metaKey;
  if (other) return null;
  if (!mod) {
    if (e.altKey && !e.shiftKey && e.key === "ArrowUp") return "moveUp";
    if (e.altKey && !e.shiftKey && e.key === "ArrowDown") return "moveDown";
    if (!e.altKey && !e.shiftKey && e.key === "Enter") return "enter";
    return null;
  }
  if (e.altKey) return null;
  if (e.key === "Enter" && !e.shiftKey) return "toggle";
  if (e.key === "Backspace" && !e.shiftKey) return "delete";
  if (e.code === "KeyZ") return e.shiftKey ? "redo" : "undo";
  if (e.code === "KeyY" && !mac && !e.shiftKey) return "redo";
  if (e.code === "KeyR" && e.shiftKey) return "template";
  const region = REGIONS[e.code];
  return region !== undefined && !e.shiftKey ? region : null;
}
