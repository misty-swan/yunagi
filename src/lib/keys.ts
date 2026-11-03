// 画面の骨組みのキー（操作仕様書 2章。初期割当。割当の変更は設定の課題で作る）と、その表示。
//
// - `Mod` は macOS では ⌘、Windows・Linux では Ctrl。
// - IME の変換中のキーはショートカットとして扱わない（操作仕様書 3章）。

/** 画面の骨組みが受け持つ操作。 */
export type ShellAction =
  | "toggleMode"
  | "prevDay"
  | "nextDay"
  | "today"
  | "palette"
  | "settings"
  | "sidebar"
  | "archive";

interface Binding {
  /**
   * 文字のキーは `KeyboardEvent.code`（配列によらず同じ位置のキー。ロシア語の配列などでも動く）。
   * 記号のキーは `KeyboardEvent.key`（押した文字。JIS 配列では `[` `]` `\` の位置が US 配列と
   * ずれ、`code` では別のキーに当たるため）。
   */
  match: { code: string } | { keys: readonly string[] };
  shift: boolean;
  /** 表示するキーの文字。 */
  key: string;
}

const BINDINGS: Record<ShellAction, Binding> = {
  toggleMode: { match: { code: "KeyJ" }, shift: false, key: "J" },
  prevDay: { match: { keys: ["["] }, shift: false, key: "[" },
  nextDay: { match: { keys: ["]"] }, shift: false, key: "]" },
  today: { match: { code: "KeyT" }, shift: false, key: "T" },
  palette: { match: { code: "KeyK" }, shift: false, key: "K" },
  settings: { match: { keys: [","] }, shift: false, key: "," },
  // JIS 配列の macOS では、\ のキーが ¥ を打つ（既定の設定）
  sidebar: { match: { keys: ["\\", "¥"] }, shift: false, key: "\\" },
  archive: { match: { code: "KeyL" }, shift: true, key: "L" },
};

/** macOS か（WebView の利用者エージェントから決める）。 */
export function isMacPlatform(nav: Pick<Navigator, "userAgent"> = navigator): boolean {
  return /Mac|iPhone|iPad/.test(nav.userAgent);
}

/** 操作のキーの表示（macOS は `⇧⌘L`、Windows・Linux は `Ctrl+Shift+L`）。 */
export function keyLabel(action: ShellAction, mac: boolean): string {
  const b = BINDINGS[action];
  return mac ? `${b.shift ? "⇧" : ""}⌘${b.key}` : `Ctrl+${b.shift ? "Shift+" : ""}${b.key}`;
}

/** Mod と Enter（完了・解消の切替）の表示。 */
export function modEnterLabel(mac: boolean): string {
  return mac ? "⌘↵" : "Ctrl+Enter";
}

/** Mod と Shift と文字のキーの表示（ノートモードの送る操作など、骨組みが受け持たない操作）。 */
export function modShiftLabel(key: string, mac: boolean): string {
  return mac ? `⇧⌘${key}` : `Ctrl+Shift+${key}`;
}

type KeyLike = Pick<
  KeyboardEvent,
  "code" | "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey" | "isComposing" | "keyCode"
>;

/** キーの押下が、骨組みの操作のどれに当たるか。当たらなければ `null`。 */
export function shellActionOf(e: KeyLike, mac: boolean): ShellAction | null {
  // IME の変換中（WebKit 系では keyCode 229 で届くことがある）は扱わない
  if (e.isComposing || e.keyCode === 229) return null;
  const mod = mac ? e.metaKey : e.ctrlKey;
  const other = mac ? e.ctrlKey : e.metaKey;
  if (!mod || other || e.altKey) return null;
  for (const [action, b] of Object.entries(BINDINGS) as [ShellAction, Binding][]) {
    if (b.shift !== e.shiftKey) continue;
    if ("code" in b.match ? e.code === b.match.code : b.match.keys.includes(e.key)) return action;
  }
  return null;
}
