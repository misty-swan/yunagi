// ショートカットの割当：操作の一覧と初期割当、割当の文字列の読み書き、キーの押下との照合、割当の検査
// （操作仕様書 1章・2章、データモデル設計書 3章 setting の `shortcuts`）。
//
// - 割当の文字列は `Mod+Shift+L`・`Alt+[`・`Mod+Enter` の形（修飾キーを `+` でつなぎ、最後にキー）。
//   読むときは並びを問わず、書くときは `Mod`・`Alt`・`Shift` の順にそろえる（`format`）。
// - 設定には、初期割当から変えたものだけを持つ。初期割当と同じ割当は持たない（`withBinding`。持つと、
//   Vim モードの有無で変わる初期割当に従わなくなるため）。
// - 純粋な関数だけで、設定の値は引数で受ける。画面が使う入口は keys.ts・board_keys.ts。
// - IME の変換中のキーはショートカットとして扱わない（操作仕様書 3章）。呼ぶ側が先に除く。

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

/** ノートモードの部品が受け持つ操作：時刻付き見出しの挿入と、TODO・障害への行送り。 */
export type NoteAction = "timeHeading" | "toTodo" | "toBlocker";

/** 整理モードの一覧と振り返り欄の操作のうち、割当を変えられるもの。 */
export type BoardAssigned =
  | "toggle"
  | "delete"
  | "moveUp"
  | "moveDown"
  | "regionTodo"
  | "regionBlocker"
  | "regionReflection"
  | "template";

/**
 * OS 全体の操作：Yunagi を前面に呼び出す。OS に登録して受ける（画面のキー処理では受けない。
 * src-tauri/src/global_shortcut.rs、`src/lib/global_shortcut.svelte.ts`）。
 */
export type GlobalAction = "bringToFront";

/** 割当を変えられる操作。 */
export type ActionId = GlobalAction | ShellAction | NoteAction | BoardAssigned;

/**
 * 操作が効く範囲：`global` は OS 全体（アプリが前面になくても）、`common` はアプリ内のすべての画面、
 * `board` は整理モード（一覧と振り返り欄）、`note` はノートモード。`global` と `common` はどれとも重なり
 * （OS に登録したキーは、Yunagi が前面にあるときも OS が先に受けるため）、`board` と `note` は重ならない。
 */
export type Scope = "global" | "common" | "board" | "note";

interface ActionDef {
  id: ActionId;
  scope: Scope;
  /** 初期割当（操作仕様書 2章）。 */
  initial: string;
}

/** 割当を変えられる操作と初期割当（設定画面に出す順）。 */
export const ACTIONS: readonly ActionDef[] = [
  { id: "bringToFront", scope: "global", initial: "Mod+Shift+D" },
  { id: "toggleMode", scope: "common", initial: "Mod+J" },
  { id: "prevDay", scope: "common", initial: "Mod+[" },
  { id: "nextDay", scope: "common", initial: "Mod+]" },
  { id: "today", scope: "common", initial: "Mod+T" },
  { id: "palette", scope: "common", initial: "Mod+K" },
  { id: "settings", scope: "common", initial: "Mod+," },
  { id: "sidebar", scope: "common", initial: "Mod+\\" },
  { id: "archive", scope: "common", initial: "Mod+Shift+L" },
  { id: "regionTodo", scope: "board", initial: "Mod+1" },
  { id: "regionBlocker", scope: "board", initial: "Mod+2" },
  { id: "regionReflection", scope: "board", initial: "Mod+3" },
  { id: "toggle", scope: "board", initial: "Mod+Enter" },
  { id: "moveUp", scope: "board", initial: "Alt+ArrowUp" },
  { id: "moveDown", scope: "board", initial: "Alt+ArrowDown" },
  { id: "delete", scope: "board", initial: "Mod+Backspace" },
  { id: "template", scope: "board", initial: "Mod+Shift+R" },
  { id: "toTodo", scope: "note", initial: "Mod+Shift+T" },
  { id: "toBlocker", scope: "note", initial: "Mod+Shift+B" },
  { id: "timeHeading", scope: "note", initial: "Mod+Shift+H" },
];

const SCOPE_OF = new Map(ACTIONS.map((a) => [a.id, a.scope]));

/** 範囲 `scope` の操作の名前（`T` はその範囲の操作の型。範囲と型が合うのは `ACTIONS` の並びで保つ）。 */
export function idsOf<T extends ActionId>(scope: Scope): readonly T[] {
  return ACTIONS.filter((a) => a.scope === scope).map((a) => a.id as T);
}

/** 操作の効く範囲どうしが重なるか。 */
function overlaps(a: Scope, b: Scope): boolean {
  return a === "global" || b === "global" || a === "common" || b === "common" || a === b;
}

/**
 * Vim モードで初期割当が変わる操作（Windows・Linux だけ。Ctrl+[ が Vim の Esc と重なるため。
 * 操作仕様書 4.1）。ふりかえりの週の移動も、前日・翌日の操作と同じ割当に従う。
 */
const VIM_INITIAL: Partial<Record<ActionId, string>> = { prevDay: "Alt+[", nextDay: "Alt+]" };

/** いまの Vim モードと OS での、操作の初期割当。 */
export function initialBinding(id: ActionId, vim: boolean, mac: boolean): string {
  const vimOne = vim && !mac ? VIM_INITIAL[id] : undefined;
  return vimOne ?? ACTIONS.find((a) => a.id === id)!.initial;
}

// ---- 割当の文字列 ----

/** 名前で持つキー。 */
const NAMED = ["Enter", "Backspace", "Delete", "Tab", "Space", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"] as const;

/** 記号のキーと、US 配列での位置（押した文字が当たらないときの代わり）。 */
const SYMBOLS: Readonly<Record<string, string>> = {
  "[": "BracketLeft",
  "]": "BracketRight",
  "\\": "Backslash",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
  ";": "Semicolon",
  "'": "Quote",
  "-": "Minus",
  "=": "Equal",
  "`": "Backquote",
};

/** 記号のキーか（`toString` のような、オブジェクトが元から持つ名前は含めない）。 */
const isSymbol = (s: string): boolean => Object.prototype.hasOwnProperty.call(SYMBOLS, s);

/** JIS 配列の macOS で `\` のキーが打つ文字。 */
const YEN = "¥";

type KeySpec =
  /** 文字・数字のキー。キーの位置（`code`）で判定する（配列によらず同じ位置のキーで動く）。 */
  | { kind: "code"; code: string; text: string }
  /** 名前で持つキー（押した `key` で判定する）。 */
  | { kind: "named"; name: (typeof NAMED)[number] }
  /** 記号のキー（押した文字で判定する。JIS 配列では位置が US 配列とずれるため）。 */
  | { kind: "symbol"; text: string };

/** 読んだ割当。 */
export interface Binding {
  mod: boolean;
  alt: boolean;
  shift: boolean;
  key: KeySpec;
}

/** 割当の文字列を読む。形が合わなければ `null`。 */
export function parseBinding(text: string): Binding | null {
  const parts = text.split("+");
  const last = parts.pop();
  if (last === undefined || last === "") return null;
  const mods = new Set<string>();
  for (const p of parts) {
    if (!(p === "Mod" || p === "Alt" || p === "Shift") || mods.has(p)) return null;
    mods.add(p);
  }
  let key: KeySpec;
  if (/^[A-Z]$/.test(last)) key = { kind: "code", code: `Key${last}`, text: last };
  else if (/^[0-9]$/.test(last)) key = { kind: "code", code: `Digit${last}`, text: last };
  else if ((NAMED as readonly string[]).includes(last)) key = { kind: "named", name: last as (typeof NAMED)[number] };
  else if (isSymbol(last)) key = { kind: "symbol", text: last };
  else return null;
  return { mod: mods.has("Mod"), alt: mods.has("Alt"), shift: mods.has("Shift"), key };
}

function keyText(k: KeySpec): string {
  return k.kind === "named" ? k.name : k.text;
}

/** 割当を、並びをそろえた文字列にする（保存する形。割当どうしの比べにも使う）。 */
export function format(b: Binding): string {
  return [b.mod && "Mod", b.alt && "Alt", b.shift && "Shift", keyText(b.key)].filter(Boolean).join("+");
}

/** 割当の文字列を、そろえた形にする。読めなければ `null`。 */
export function canonical(text: string): string | null {
  const b = parseBinding(text);
  return b === null ? null : format(b);
}

const MAC_NAMED: Record<string, string> = { Enter: "↵", Backspace: "⌫", Delete: "⌦", Tab: "⇥", Space: "Space", ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→" };
const PC_NAMED: Record<string, string> = { ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→" };

/** 画面に出す割当の表記（macOS は ⌥⇧⌘ の記号、Windows・Linux は Ctrl+Alt+Shift+）。 */
export function label(b: Binding, mac: boolean): string {
  const k = b.key.kind === "named" ? (mac ? MAC_NAMED : PC_NAMED)[b.key.name] ?? b.key.name : b.key.text;
  if (mac) return `${b.alt ? "⌥" : ""}${b.shift ? "⇧" : ""}${b.mod ? "⌘" : ""}${k}`;
  return [b.mod && "Ctrl", b.alt && "Alt", b.shift && "Shift", k].filter(Boolean).join("+");
}

// ---- 割当の検査 ----

/** 割り当てられない理由。 */
export type BindingProblem =
  /** 修飾キー（Mod か Alt）が無い。文字の入力と区別できない。 */
  | "no_modifier"
  /** macOS の Option だけ（文字や記号と）。Option は文字を打つキーなので、Mod と組み合わせる。 */
  | "mac_option_alone"
  /** 名前のキーのうち、文字の編集や移動に使うもの（Mod は Enter・Backspace だけ、Alt だけは上下の矢印だけ）。 */
  | "edit_key"
  /** Windows・Linux の Ctrl と Alt の組み合わせ（AltGr で文字を打つ配列があるため）。 */
  | "alt_gr"
  /** 記号のキーと Shift（配列で打つ文字が変わる）。 */
  | "shifted_symbol"
  /** macOS の Option と記号のキー（打つ文字が配列で変わる）。 */
  | "mac_option_symbol"
  /** ほかの操作が使うキー（取り消し・コピーなど）。 */
  | "reserved"
  /** OS 全体の操作に使えるのは、英字か数字のキーだけ（OS への登録はキーの位置で行うため、記号や名前のキーは配列や OS で扱いが違う）。 */
  | "global_key";

/**
 * 設定とは別に決まっている、割り当てられないキー（そろえた形）。
 * - Mod+Z・Shift・Y：取り消し・やり直し（エディタの履歴と整理モードのリストの履歴。割当を変えられない）
 * - Mod+C・V・X・A：コピー・貼り付け・切り取り・全選択
 * - Mod+Shift+A（依頼文の書き出し）：まだ作っていない操作。作るときに割当を変えられる操作（`ACTIONS`）へ
 *   移し、ここから外す
 */
const RESERVED = ["Mod+Z", "Mod+Shift+Z", "Mod+C", "Mod+V", "Mod+X", "Mod+A", "Mod+Shift+A"];

/** Windows・Linux で、Vim モードの Esc と重なるキー（Ctrl+[）。 */
const VIM_ESC = "Mod+[";

/** Windows・Linux で、Vim モードのやり直し（`<C-r>`。操作仕様書 4.2）と重なるキー（Ctrl+R）。 */
const VIM_REDO = "Mod+R";

/**
 * 割り当てられないキーか。割り当てられれば `null`。`id` は割り当てる操作（OS 全体の操作だけ、使えるキーが
 * 狭い。渡さなければ、画面内の操作として検査する）。
 */
export function problemOf(b: Binding, vim: boolean, mac: boolean, id?: ActionId): BindingProblem | null {
  if (!b.mod && !b.alt) return "no_modifier";
  if (id !== undefined && SCOPE_OF.get(id) === "global" && b.key.kind !== "code") return "global_key";
  if (!mac && b.mod && b.alt) return "alt_gr";
  if (b.key.kind === "symbol") {
    if (b.shift) return "shifted_symbol";
    if (mac && b.alt) return "mac_option_symbol";
  }
  if (mac && !b.mod && b.alt && b.key.kind !== "named") return "mac_option_alone";
  // 名前のキーで割り当てられるのは、Mod と Enter・Backspace、Alt だけと上下の矢印だけ。ほかは文字の編集や
  // 移動（Mod+左右の矢印は行頭・行末や単語の移動、Option・Alt+Backspace は単語の削除、Alt+左右は戻る・進む、
  // Mod+Space は Spotlight など）に使う
  if (b.key.kind === "named") {
    const n = b.key.name;
    const ok = b.mod ? !b.alt && (n === "Enter" || n === "Backspace") : b.alt && (n === "ArrowUp" || n === "ArrowDown");
    if (!ok) return "edit_key";
  }
  const f = format(b);
  if (RESERVED.includes(f)) return "reserved";
  if (!mac && f === "Mod+Y") return "reserved";
  if (vim && !mac && (f === VIM_ESC || f === VIM_REDO)) return "reserved";
  return null;
}

// ---- 割当の解決 ----

/** 操作の、いまの割当の状態。 */
export interface Resolved {
  /** 使える割当。使えないとき（`status` が `lost`）は `null`。 */
  binding: Binding | null;
  /** 初期割当（Vim モードと OS を当てたもの）。 */
  initial: string;
  /** 初期割当から変えているか。 */
  changed: boolean;
  /**
   * `ok`：割当どおり。`lost`：同じキーをほかの操作が割り当てて使っていて、この操作は使えない。
   * `ignored`：設定にあった割当が読めない・割り当てられないキーなので、初期割当にしている。
   */
  status: "ok" | "lost" | "ignored";
  /** `lost` のとき、キーを使っているほかの操作。 */
  takenBy?: ActionId;
}

export type Keymap = ReadonlyMap<ActionId, Resolved>;

/**
 * 設定の割当（初期割当から変えたものだけ）から、すべての操作のいまの割当を求める。
 * 同じキーを重なる範囲の2つの操作が割り当てるときは、変えた割当が初期割当に勝ち（初期割当の側は使えない）、
 * 変えた割当どうしは設定画面の並びの先のものが勝つ（画面からは同じキーを2つに割り当てられないが、
 * Vim モードで初期割当が変わったときと、設定を直接書き換えたときに起こる）。
 */
export function resolveKeymap(shortcuts: Readonly<Record<string, string>>, vim: boolean, mac: boolean): Keymap {
  const out = new Map<ActionId, Resolved>();
  const holder = new Map<string, ActionId[]>();
  const takenBy = (id: ActionId, key: string): ActionId | undefined =>
    holder.get(key)?.find((o) => overlaps(SCOPE_OF.get(o)!, SCOPE_OF.get(id)!));
  const take = (id: ActionId, key: string) => holder.set(key, [...(holder.get(key) ?? []), id]);

  const rest: ActionDef[] = [];
  for (const a of ACTIONS) {
    const initial = initialBinding(a.id, vim, mac);
    const stored = shortcuts[a.id];
    const parsed = stored === undefined ? null : parseBinding(stored);
    if (stored === undefined || (parsed !== null && format(parsed) === initial)) {
      rest.push(a);
      continue;
    }
    if (parsed === null || problemOf(parsed, vim, mac, a.id) !== null) {
      out.set(a.id, { binding: null, initial, changed: false, status: "ignored" });
      rest.push(a);
      continue;
    }
    const key = format(parsed);
    const other = takenBy(a.id, key);
    if (other !== undefined) {
      out.set(a.id, { binding: null, initial, changed: true, status: "lost", takenBy: other });
      continue;
    }
    take(a.id, key);
    out.set(a.id, { binding: parsed, initial, changed: true, status: "ok" });
  }
  for (const a of rest) {
    const initial = initialBinding(a.id, vim, mac);
    const previous = out.get(a.id);
    const other = takenBy(a.id, initial);
    if (other !== undefined) {
      out.set(a.id, { binding: null, initial, changed: false, status: "lost", takenBy: other });
      continue;
    }
    take(a.id, initial);
    out.set(a.id, { binding: parseBinding(initial), initial, changed: false, status: previous?.status ?? "ok" });
  }
  return out;
}

/**
 * `id` に `binding` を割り当てると、重なる範囲のどの操作と同じキーになるか（いま使えている割当と比べる）。
 * 重ならなければ `null`。
 */
export function conflictOf(id: ActionId, binding: Binding, keymap: Keymap): ActionId | null {
  const key = format(binding);
  for (const a of ACTIONS) {
    if (a.id === id || !overlaps(a.scope, SCOPE_OF.get(id)!)) continue;
    const r = keymap.get(a.id);
    if (r === undefined) continue;
    // 変えていない割当が、ほかの操作に取られて使えないときも、その初期割当のキーは譲らない
    // （取っていた操作を変えたとき、黙って使えるようになるため、先に断る）
    const held = r.binding !== null ? format(r.binding) : r.changed ? null : r.initial;
    if (held === key) return a.id;
  }
  return null;
}

/**
 * `id` の割当を `binding` にした割当の一覧（新しい値。元は変えない）。`binding` が初期割当と同じ、または
 * `null`（初期割当に戻す）なら、`id` の割当を持たない。
 */
export function withBinding(
  shortcuts: Readonly<Record<string, string>>,
  id: ActionId,
  binding: Binding | null,
  vim: boolean,
  mac: boolean,
): Record<string, string> {
  const next = { ...shortcuts };
  if (binding === null || format(binding) === initialBinding(id, vim, mac)) delete next[id];
  else next[id] = format(binding);
  return next;
}

// ---- キーの押下との照合 ----

/** 照合に使うキーの押下の情報。 */
export type KeyLike = Pick<KeyboardEvent, "code" | "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey"> & {
  getModifierState?: KeyboardEvent["getModifierState"];
};

/** キーの押下が割当に当たるか（IME の変換中かの判定は、呼ぶ側が先に行う）。 */
export function matches(e: KeyLike, mac: boolean, b: Binding): boolean {
  const mod = mac ? e.metaKey : e.ctrlKey;
  const other = mac ? e.ctrlKey : e.metaKey;
  if (mod !== b.mod || other) return false;
  // Windows・Linux の AltGr は Ctrl+Alt として届く。文字の入力かもしれないので、操作として扱わない
  // （ドイツ語の配列の AltGr+8 で [ を打つとき、前日へ移ってしまわないように）
  if (!mac && (e.getModifierState?.("AltGraph") === true || (e.ctrlKey && e.altKey))) return false;
  const k = b.key;
  if (k.kind === "code") return e.code === k.code && e.shiftKey === b.shift && e.altKey === b.alt;
  if (k.kind === "named") {
    const name = k.name === "Space" ? " " : k.name;
    return e.key === name && e.shiftKey === b.shift && e.altKey === b.alt;
  }
  // 記号のキー：macOS で記号を Option と打つ配列（ドイツ語の配列の Option+5 で [ など）でも動くよう、
  // 押した文字が合えば Option と Shift は問わない。Windows・Linux では Alt の有無を割当に合わせる
  if (!mac && e.altKey !== b.alt) return false;
  if (e.key === k.text || (k.text === "\\" && e.key === YEN)) {
    // 押した文字が変わらない配列で、そのキー自身に Option・Shift を重ねたもの（⌘⌥[・⌘⇧[ など、ほかのアプリや
    // OS が使う組み合わせ）は、割当の記号のキーとして受けない。別のキーで打つ記号（Option+5 で [）は受ける
    if (e.code === SYMBOLS[k.text] && (e.shiftKey || (mac && e.altKey))) return false;
    return true;
  }
  // 記号を単独のキーで打てない配列（ドイツ語の配列など）でも動くよう、Option の無いときは、US 配列の
  // 記号のキーの位置でも受ける。ただし、押したキーがほかの割り当てられる記号を打つときは、その記号のキーなので受けない
  if (isSymbol(e.key) || e.key === YEN) return false;
  return (mac ? !e.altKey : true) && !e.shiftKey && e.code === SYMBOLS[k.text];
}

// ---- 設定画面でのキーの取り込み ----

/** キーの押下から、割当を作る。修飾キーだけのとき・扱えないキーのときは `null`。 */
export function bindingFromEvent(e: KeyLike, mac: boolean): Binding | null {
  const mod = mac ? e.metaKey : e.ctrlKey;
  const base = { mod, alt: e.altKey, shift: e.shiftKey };
  // macOS の Control と Windows・Linux の Meta（Win キー）は割り当てない
  if (mac ? e.ctrlKey : e.metaKey) return null;
  let key: KeySpec | null = null;
  const letter = /^Key([A-Z])$/.exec(e.code);
  const digit = /^Digit([0-9])$/.exec(e.code);
  if (letter) key = { kind: "code", code: e.code, text: letter[1] };
  else if (digit) key = { kind: "code", code: e.code, text: digit[1] };
  else {
    const name = e.key === " " ? "Space" : e.key;
    if ((NAMED as readonly string[]).includes(name)) key = { kind: "named", name: name as (typeof NAMED)[number] };
    else {
      const text = e.key === YEN ? "\\" : e.key;
      if (isSymbol(text)) key = { kind: "symbol", text };
      else if (!/^[\x20-\x7e]$/.test(e.key)) {
        // 押した文字が ASCII でないとき（ドイツ語の配列の ü など）は、US 配列での位置から決める。ASCII の
        // 記号を打つキー（JIS 配列の @ など）は、位置では決めない（別のキーとして記録してしまうため）
        const at = Object.entries(SYMBOLS).find(([, code]) => code === e.code);
        if (at) key = { kind: "symbol", text: at[0] };
      }
    }
  }
  return key === null ? null : { ...base, key };
}
