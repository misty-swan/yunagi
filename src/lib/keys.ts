// 画面の骨組みのキー（操作仕様書 2章）と、その表示。割当は初期割当に、設定で変えたもの
// （`prefs.shortcuts`）と Vim モード（`prefs.vimMode`）を当てたもの（keymap.ts）。割当の値は呼ぶたびに
// 読む（設定を変えたら、次のキーの押下とキーの案内にすぐ当たる）。
//
// - `Mod` は macOS では ⌘、Windows・Linux では Ctrl。
// - IME の変換中のキーはショートカットとして扱わない（操作仕様書 3章）。

import {
  idsOf,
  label,
  matches,
  resolveKeymap,
  type ActionId,
  type Keymap,
  type KeyLike as MatchKey,
  type NoteAction,
  type ShellAction,
} from "./keymap";
import { prefs } from "./prefs.svelte";

export type { NoteAction, ShellAction };

/** macOS か（WebView の利用者エージェントから決める）。 */
export function isMacPlatform(nav: Pick<Navigator, "userAgent"> = navigator): boolean {
  return /Mac|iPhone|iPad/.test(nav.userAgent);
}

/** `mac` ごとの、求めた割当（呼び出し側が違う `mac` で呼んでも、互いに追い出さない）。 */
const cache = new Map<boolean, { shortcuts: unknown; vim: boolean; map: Keymap }>();

/** いまの割当（設定を当てたもの）。同じ設定と OS で続けて呼ぶ間は、求め直さない。 */
export function currentKeymap(mac: boolean): Keymap {
  const shortcuts = prefs.shortcuts;
  const vim = prefs.vimMode;
  const hit = cache.get(mac);
  if (hit !== undefined && hit.shortcuts === shortcuts && hit.vim === vim) return hit.map;
  const map = resolveKeymap(shortcuts, vim, mac);
  cache.set(mac, { shortcuts, vim, map });
  return map;
}

/** 操作の割当の表記（`Ctrl+Shift+L`・`⇧⌘L`）。使えない割当のときは空。 */
export function keyLabel(action: ActionId, mac: boolean): string {
  const b = currentKeymap(mac).get(action)?.binding;
  return b === null || b === undefined ? "" : label(b, mac);
}

type KeyLike = MatchKey & Pick<KeyboardEvent, "isComposing" | "keyCode">;

/** IME の変換中か（WebKit 系では keyCode 229 で届くことがある）。 */
export function composing(e: Pick<KeyboardEvent, "isComposing" | "keyCode">): boolean {
  return e.isComposing || e.keyCode === 229;
}

/** キーの押下が、`ids` のうちどの操作に当たるか。当たらなければ `null`。 */
export function actionIn<T extends ActionId>(e: KeyLike, mac: boolean, ids: readonly T[]): T | null {
  if (composing(e)) return null;
  const map = currentKeymap(mac);
  for (const id of ids) {
    const b = map.get(id)?.binding;
    if (b !== null && b !== undefined && matches(e, mac, b)) return id;
  }
  return null;
}

const SHELL_IDS = idsOf<ShellAction>("common");

/** キーの押下が、骨組みの操作のどれに当たるか。当たらなければ `null`。 */
export function shellActionOf(e: KeyLike, mac: boolean): ShellAction | null {
  return actionIn(e, mac, SHELL_IDS);
}

const NOTE_IDS = idsOf<NoteAction>("note");

/**
 * キーの押下が、ノートモードの操作に当たるか（初期割当は Mod+Shift+H・T・B。操作仕様書 2章）。
 * 当たらなければ `null`。
 */
export function noteActionOf(e: KeyLike, mac: boolean): NoteAction | null {
  return actionIn(e, mac, NOTE_IDS);
}
