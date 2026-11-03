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
   * ずれ、`code` では別のキーに当たるため）。`shift` は文字のキーにだけ使う。
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

/** 記号のキーの、US 配列での位置（押した文字が当たらなかったときに使う）。 */
const SYMBOL_POSITIONS: Partial<Record<string, ShellAction>> = {
  BracketLeft: "prevDay",
  BracketRight: "nextDay",
  Backslash: "sidebar",
  Comma: "settings",
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
  if (!mod || other) return null;
  // Windows・Linux の Alt 付き（AltGr は Ctrl+Alt として届く）は、文字の入力かもしれないので扱わない
  // （ドイツ語の配列の AltGr+8 で [ を打つとき、前日へ移ってしまわないように）
  if (!mac && e.altKey) return null;
  const bindings = Object.entries(BINDINGS) as [ShellAction, Binding][];
  for (const [action, b] of bindings) {
    if ("code" in b.match) {
      if (e.code === b.match.code && b.shift === e.shiftKey && !e.altKey) return action;
    } else if (b.match.keys.includes(e.key)) {
      // macOS で記号を Option と打つ配列（ドイツ語の配列の Option+5 で [ など）でも動くよう、
      // 押した文字が合えば Option と Shift は問わない
      return action;
    }
  }
  // 記号を単独のキーで打てない配列（ドイツ語の配列など）でも動くよう、Alt の無いときは、
  // US 配列の記号のキーの位置でも受ける
  const fallback = e.altKey ? undefined : SYMBOL_POSITIONS[e.code];
  return fallback !== undefined && !e.shiftKey ? fallback : null;
}
