// ノート・振り返りのエディタの Vim のキー（操作仕様書 4.1・4.2）。Vim の動きは `@replit/codemirror-vim`
// （CodeMirror 6 の Vim 拡張。アーキテクチャ設計書 3章）に任せ、ここでは Yunagi に合わせる部分だけを足す。
//
// - Mod を使う標準のショートカットを Vim が横取りしない：Vim は押したキーを使うと、それ以上キーを
//   渡さない（アプリ全体のキー処理に届かない）。Windows・Linux の Mod は Ctrl なので、Vim の Ctrl のキー
//   （`<C-v>` の矩形選択、`<C-a>` `<C-x>` の数の増減、`<C-d>` `<C-u>` のスクロール、挿入モードの `<C-w>` など）
//   は、エディタが押下を Vim に渡す関数（`Vim.vimKeyFromEvent`）を包んで、Vim に渡さない（検索欄などの
//   Vim の内部の処理は、この関数を通らないので変わらない）。受けるのは `<C-r>`（やり直し。4.2）と
//   `<C-[>`（Esc）だけ。macOS の Mod は ⌘ で Vim は使わないので、そのまま。
// - Yunagi 固有のキー `<Space>t` `<Space>b` `<Space>h`（ノートだけ。4.2）：Vim の対応表は全体で1つなので、
//   読み込み時に一度だけ登録し、動かすエディタはその時のエディタ（`setVimHost`）で決める。振り返り欄は
//   何も登録しない（キーは受けて、何も起こさない。登録が無いと `<Space>` が右への移動、`t` が文字の検索に
//   なってしまう）。
// - 現在のモードは、画面の部品が小さく出す（Vim 拡張の下端の帯は使わない）。`onMode` で知らせる。

import { Facet, type Extension } from "@codemirror/state";
import { EditorView, ViewPlugin } from "@codemirror/view";
import { getCM, Vim, vim } from "@replit/codemirror-vim";
import type { ItemKind } from "./bindings";
import { imeEnterInsert, imeLeaveInsert } from "./ime.svelte";
import { composing, isMacPlatform, vimKeyOfCode } from "./keys";

/** Vim のモード（操作仕様書 4.1 の表示）。 */
export type VimMode = "normal" | "insert" | "visual";

/** ノートが Vim のキーに応える操作（振り返り欄は持たない）。 */
export interface VimHost {
  /** 現在行（ビジュアルモードでは選択した行）を TODO・障害へ送る。送り終えたら解決する。 */
  send(v: EditorView, kind: ItemKind): Promise<void>;
  /** 時刻付き見出しを入れる（カーソルは見出しの後ろ）。入れられなかった（入力を受け付けていない）ら偽。 */
  timeHeading(v: EditorView): boolean;
}

const hosts = new WeakMap<EditorView, VimHost>();

/** エディタ `v` が `<Space>t` `<Space>b` `<Space>h` に応える操作を決める（`null` で何もしない）。 */
export function setVimHost(v: EditorView, host: VimHost | null) {
  if (host === null) hosts.delete(v);
  else hosts.set(v, host);
}

/** Vim が使う Ctrl のキー（Windows・Linux で Mod と重なるが、やり直しと Esc は 4.2 の表にあるので使う）。 */
const VIM_CTRL_KEYS = ["r", "["];

/**
 * Vim に渡さない押下か：Windows・Linux で Ctrl を押している（AltGr の Ctrl+Alt は文字の入力なので除く）、
 * `<C-r>` と `<C-[>` 以外。Vim の `<C-v>` `<C-a>` `<C-x>` `<C-d>` `<C-w>` などは、Mod を使う標準の
 * ショートカットと重なる。
 */
export function leavesToApp(e: Pick<KeyboardEvent, "ctrlKey" | "altKey" | "shiftKey" | "key">, mac: boolean): boolean {
  // Ctrl+Shift+R（振り返りのテンプレート）などの Shift 付きも、アプリのキーなので Vim に渡さない
  // （渡すと、続きのキー待ちの `d` や回数が黙って捨てられる）
  return !mac && e.ctrlKey && !e.altKey && (e.shiftKey || !VIM_CTRL_KEYS.includes(e.key.toLowerCase()));
}

let ready = false;

/**
 * Vim の対応表を Yunagi に合わせる（一度だけ。対応表は全体で1つ）。
 *
 * Vim の割当を外す（`Vim.unmap`）と、Vim の内部の長さの記録とずれて、キーの照合が壊れる。そのため、
 * 押下から Vim のキーの名前を作る関数を包み、Vim に渡さない押下は名前を作らない（Vim は無視する）。
 */
function setup() {
  if (ready) return;
  ready = true;
  const mac = isMacPlatform();
  const keyOf = Vim.vimKeyFromEvent;
  Vim.vimKeyFromEvent = (e, state) => {
    if (leavesToApp(e, mac)) return undefined;
    // 挿入モードの変換中の Esc は変換の取り消しなので、Vim の Esc として扱わない（操作仕様書 3章）。除くのは
    // 挿入モードだけ（ノーマルモード中は、IME の状態にかかわらずキーを Vim の操作として扱う。4.4 の 2。
    // `imeKeydown`）。Vim が自分で作る、押下の情報を持たない擬似の押下は、どちらの項目も無いので除かれない
    if (state?.insertMode === true && composing(e)) return undefined;
    // Caps Lock が入っていると Ctrl+R の `key` は "R" で届き、Vim の `<C-R>`（割当なし）になってしまう
    if (e.ctrlKey && !e.shiftKey && e.key.length === 1 && e.key !== e.key.toLowerCase()) {
      const { ctrlKey, altKey, metaKey, shiftKey, code } = e;
      return keyOf({ ctrlKey, altKey, metaKey, shiftKey, code, key: e.key.toLowerCase() } as KeyboardEvent, state);
    }
    return keyOf(e, state);
  };
  // `<Space>`（右へ移動。4.2 の表には無い）は、押した時点で `l` に当たってしまい、`<Space>t` などの続きを
  // 待たない。この標準の割当だけを外す（1つだけなら、Vim の内部の記録とずれない）
  Vim.unmap("<Space>", undefined as never);
  const act = (name: string, run: (v: EditorView, cm: unknown) => void) =>
    Vim.defineAction(name, (cm) => run((cm as { cm6: EditorView }).cm6, cm));
  /** ビジュアルモードのとき、送り終えたら抜ける（送る行は選択範囲で決めるので、送り終えるまで抜けない）。 */
  const send = (kind: ItemKind) => (v: EditorView, cm: unknown) => {
    const host = hosts.get(v);
    if (host === undefined) return;
    const visual = getVimState(cm)?.visualMode === true;
    void host.send(v, kind).catch(() => {}).finally(() => {
      // 送っている間に日を移った・Vim モードを切り替えたなら、Vim が作り直されている（古いほうは触らない）
      if (visual && getCM(v) === cm && getVimState(cm)?.visualMode === true) Vim.exitVisualMode(cm as never);
    });
  };
  act("yunagiToTodo", send("todo"));
  act("yunagiToBlocker", send("blocker"));
  act("yunagiTimeHeading", (v, cm) => {
    const host = hosts.get(v);
    if (host === undefined) return;
    // 見出しを入れてから、行末で挿入モードに入る（続けて件名を書ける）
    if (host.timeHeading(v)) Vim.handleKey(cm as never, "A", "user");
  });
  for (const context of ["normal", "visual"]) {
    Vim.mapCommand("<Space>t", "action", "yunagiToTodo", {}, { context });
    Vim.mapCommand("<Space>b", "action", "yunagiToBlocker", {}, { context });
  }
  Vim.mapCommand("<Space>h", "action", "yunagiTimeHeading", {}, { context: "normal" });
  // ビジュアルモードの `<Space>h` は何もしない（見出しは選択範囲に入れない）
  Vim.mapCommand("<Space>h", "action", "yunagiNothing", {}, { context: "visual" });
  Vim.defineAction("yunagiNothing", () => {});
}

function getVimState(cm: unknown): { visualMode: boolean; insertMode: boolean } | undefined {
  return (cm as { state?: { vim?: { visualMode: boolean; insertMode: boolean } } }).state?.vim;
}

/** エディタ `v` の Vim のモード（Vim が無効なら `null`）。 */
export function vimModeOf(v: EditorView): VimMode | null {
  const cm = getCM(v);
  const s = cm === null ? undefined : getVimState(cm);
  if (s === undefined) return null;
  return s.insertMode ? "insert" : s.visualMode ? "visual" : "normal";
}

/**
 * ノーマルモードで、Vim が続きのキーを何も待っていないか（回数・レジスタ・オペレータ・`f` `t` `r` などの
 * 文字待ちがない）。待っている間の `]` は Vim の入力（`f]`・`r]`・`d]` など）なので、領域の移動
 * （`]]` `[[`。4.3）に使わない。
 */
export function vimIdle(v: EditorView): boolean {
  const cm = getCM(v);
  const s = cm === null ? undefined : (cm as unknown as { state?: { vim?: { insertMode: boolean; visualMode: boolean; inputState?: InputLike } } }).state?.vim;
  if (s === undefined || s.insertMode || s.visualMode) return false;
  const i = s.inputState;
  if (i === undefined) return true;
  return (
    i.keyBuffer.length === 0 &&
    i.prefixRepeat.length === 0 &&
    i.motionRepeat.length === 0 &&
    !i.operator &&
    !i.motion &&
    !i.registerName
  );
}

interface InputLike {
  keyBuffer: string[];
  prefixRepeat: string[];
  motionRepeat: string[];
  operator?: string | null;
  motion?: string | null;
  registerName?: string;
}

/** モードの変化を知らせる先。 */
export const onVimMode = Facet.define<(mode: VimMode) => void, readonly ((mode: VimMode) => void)[]>({
  combine: (all) => all,
});

/** Vim のモードの変化を聞く（モードが変わるたびに `onVimMode` へ知らせる）。 */
const modeListener = ViewPlugin.fromClass(
  class {
    readonly #off: (() => void) | null;
    #mode: () => VimMode | null = () => null;
    constructor(v: EditorView) {
      const cm = getCM(v);
      let last: VimMode | null = null;
      this.#mode = () => last;
      // ビジュアルモードを抜けるときなどに同じモードを続けて知らせてくるので、変わったときだけ伝える
      const tell = (mode: VimMode) => {
        if (mode === last) return;
        const before = last;
        last = mode;
        // 挿入モードの出入りに合わせて IME を切り替える（操作仕様書 4.4。最初の通知は出入りでない）
        if (before === "insert" && mode !== "insert") imeLeaveInsert();
        else if (before !== null && before !== "insert" && mode === "insert") imeEnterInsert();
        v.state.facet(onVimMode).forEach((f) => f(mode));
      };
      if (cm === null) {
        this.#off = null;
        return;
      }
      // 上書き（R）の replace は、文字を入れるモードなので INSERT として出す（`vimModeOf` と同じ）
      const on = (e: { mode: string }) =>
        tell(e.mode === "insert" || e.mode === "replace" ? "insert" : e.mode === "visual" ? "visual" : "normal");
      cm.on("vim-mode-change", on);
      this.#off = () => cm.off("vim-mode-change", on);
      tell(vimModeOf(v) ?? "normal");
    }
    destroy() {
      this.#off?.();
      // 挿入モードのまま閉じた（日を移った・エディタを作り直したなど）ときも、抜けたものとして扱う
      // （ノーマルモードで始まる新しいエディタに、日本語入力が残らないように）
      if (this.#mode() === "insert") imeLeaveInsert();
    }
  },
);

/**
 * ノーマル・ビジュアルモードの Space を、本文に入れない。Vim は、続きのキーを待っている間（`d` `g` `z` や
 * 自分で足した `<Space>` の後）に Space が来て、どの割当にも当たらないと、そのキーを使わずに返す。すると
 * 空白が本文に入ってしまう。Vim が使わなかった Space だけをここで止める（挿入モードの Space は文字なので通す）。
 */
const keepSpaceOut = EditorView.domEventHandlers({
  keydown: (e, v) => e.key === " " && !composing(e) && !e.ctrlKey && !e.metaKey && !e.altKey && vimModeOf(v) !== "insert",
});

/**
 * ノーマル・ビジュアルモード中に、IME が先に受けたキー（`key` が "Process" など）の、押した物理キーから
 * 作った Vim のキー。IME の状態にかかわらずキーを Vim の操作として扱うため（操作仕様書 4.4 の 2）。
 *
 * Vim 拡張は、このキーの押下では何もせず、IME が入れようとした文字（変換が始まったときの1文字）を
 * Vim のキーとして扱って、変換を終わらせる。この文字は、かなの入力ではローマ字の母音が「あ」「い」に
 * なる（`a` `i` `o` が Vim のキーにならない）。そこで、押下のときに物理キーから Vim のキーを作って
 * 覚え、入力の文字を、その Vim のキーに置き換えて Vim 拡張に渡す。
 */
const imeKeys = new WeakMap<EditorView, { key: string; at: number }[]>();

/** 覚えたキーを使える時間（ミリ秒）。IME の入力は押下の直後に届くので、長く持たない（届かなかった押下の
 * キーが、後の貼り付けなどの入力を置き換えないため）。 */
const IME_KEY_TTL = 500;

/** IME が先に受けたキーの押下か（`key` が実際のキーでない）。実際のキーで届くときは、Vim 拡張が自分で扱う。 */
function processKey(e: KeyboardEvent): boolean {
  return e.key === "Process" || e.key === "Unidentified";
}

const imeKeydown = EditorView.domEventHandlers({
  keydown: (e, v) => {
    // 実際のキーの押下が来たら、待たせていたキーは捨てる。IME が受けたキーは、入力が届く前に続けて
    // 押されることがあるので、押した順に溜める
    if (!processKey(e)) imeKeys.delete(v);
    const mode = vimModeOf(v);
    if (mode !== null && mode !== "insert" && processKey(e)) {
      const key = vimKeyOfCode(e);
      const now = Date.now();
      const queue = (imeKeys.get(v) ?? []).filter((k) => now - k.at <= IME_KEY_TTL);
      if (key !== null) queue.push({ key, at: now });
      imeKeys.set(v, queue);
    }
    return false;
  },
});

const imeInput: Parameters<typeof EditorView.inputHandler.of>[0] = (v, from, to, text, insert) => {
  const queue = imeKeys.get(v);
  if (queue === undefined) return false;
  const now = Date.now();
  while (queue.length > 0 && now - queue[0]!.at > IME_KEY_TTL) queue.shift();
  const hit = queue.shift();
  if (hit === undefined) return false;
  const key = hit.key;
  if (vimModeOf(v) === "insert" || text === key) return false;
  // ほかの入力の処理（Vim 拡張）に、置き換えたキーを渡す
  return v.state.facet(EditorView.inputHandler).some((h) => h !== imeInput && h(v, from, to, key, insert));
};

/** Vim のキーを足す拡張（エディタの標準のキーより先に試す位置に置く）。 */
export function vimExtension(): Extension {
  setup();
  // `keepSpaceOut` は、Vim が受けなかったキーだけを見るよう、Vim の後に置く。IME のキーの置き換えは、
  // Vim 拡張の入力の処理より先に試す
  return [imeKeydown, EditorView.inputHandler.of(imeInput), vim({ status: false }), keepSpaceOut, modeListener];
}
