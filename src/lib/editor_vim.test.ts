import { history, historyKeymap, standardKeymap } from "@codemirror/commands";
import { EditorState, Prec } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { getCM, Vim } from "@replit/codemirror-vim";
import { afterEach, expect, test } from "vitest";
import { leavesToApp, onVimMode, setVimHost, vimExtension, vimModeOf, type VimHost, type VimMode } from "./editor_vim";

// jsdom には Range の位置が無い（CodeMirror が測るときに使う）
Range.prototype.getClientRects ??= () => [] as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();

let view: EditorView | undefined;
afterEach(() => {
  view?.destroy();
  view = undefined;
  document.body.innerHTML = "";
});

function make(doc: string, modes: VimMode[] = [], standard = true): EditorView {
  const parent = document.createElement("div");
  document.body.append(parent);
  view = new EditorView({
    parent,
    state: EditorState.create({
      doc,
      extensions: [
        Prec.high(vimExtension()),
        onVimMode.of((m) => modes.push(m)),
        history(),
        standard ? keymap.of([...historyKeymap, ...standardKeymap]) : [],
      ],
    }),
  });
  return view;
}

/** Vim のキーを順に押す（Vim の表記。`<Esc>` など）。 */
function keys(v: EditorView, ...ks: string[]) {
  const cm = getCM(v)!;
  for (const k of ks) for (const one of k.match(/<[^>]+>|./g)!) Vim.handleKey(cm, one, "user");
}

/** 実際の keydown を内容の要素に送る。Vim が使ったら（止めたら）真。 */
function press(v: EditorView, init: KeyboardEventInit): { consumed: boolean } {
  const e = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  v.contentDOM.dispatchEvent(e);
  return { consumed: e.defaultPrevented };
}

test("基本の移動と編集が動く", () => {
  const v = make("one two\nthree\nfour");
  keys(v, "w", "x");
  expect(v.state.doc.toString()).toBe("one wo\nthree\nfour");
  keys(v, "dd");
  expect(v.state.doc.toString()).toBe("three\nfour");
  keys(v, "u");
  expect(v.state.doc.toString()).toBe("one wo\nthree\nfour");
  keys(v, "G", "o");
  expect(vimModeOf(v)).toBe("insert");
  v.dispatch({ changes: { from: v.state.selection.main.head, insert: "five" } });
  keys(v, "<Esc>");
  expect(v.state.doc.toString()).toBe("one wo\nthree\nfour\nfive");
  expect(vimModeOf(v)).toBe("normal");
});

// 操作仕様書 4.2 の表。位置は `|` で示す（`|` の直後の文字がカーソル）。画面の配置が要る j / k は含めない
// （jsdom には配置が無く、正しく動かない）。挿入モードの入力は、ブラウザが入れるので、`ins` で直接入れる。
const TABLE: { name: string; from: string; keys: string[]; ins?: string; to: string; mode?: VimMode }[] = [
  { name: "w b e", from: "|one two three", keys: ["w", "w", "b", "e"], to: "one tw|o three" },
  { name: "0 ^ $", from: "  one |two", keys: ["$"], to: "  one tw|o" },
  { name: "0", from: "  one |two", keys: ["0"], to: "|  one two" },
  { name: "^", from: "  one |two", keys: ["^"], to: "  |one two" },
  { name: "gg G", from: "a\n|b\nc", keys: ["G", "g", "g"], to: "|a\nb\nc" },
  { name: "{ }", from: "|a\nb\n\nc\nd", keys: ["}"], to: "a\nb\n|\nc\nd" },
  { name: "{", from: "a\nb\n\nc\n|d", keys: ["{"], to: "a\nb\n|\nc\nd" },
  { name: "f t", from: "|abc,def", keys: ["f", ",", "t", "f"], to: "abc,d|ef" },
  { name: "回数 3w", from: "|a b c d e", keys: ["3", "w"], to: "a b c |d e" },
  { name: "x", from: "|abc", keys: ["x"], to: "|bc" },
  { name: "dd", from: "a\n|b\nc", keys: ["d", "d"], to: "a\n|c" },
  { name: "2dd", from: "|a\nb\nc", keys: ["2", "d", "d"], to: "|c" },
  { name: "dw", from: "|one two", keys: ["d", "w"], to: "|two" },
  { name: "cw", from: "|one two", keys: ["c", "w"], ins: "1", to: "1| two", mode: "insert" },
  { name: "cc", from: "a\n|b\nc", keys: ["c", "c"], ins: "X", to: "a\nX|\nc", mode: "insert" },
  { name: "yy p", from: "|a\nb", keys: ["y", "y", "p"], to: "a\n|a\nb" },
  { name: "yy P", from: "a\n|b", keys: ["y", "y", "P"], to: "a\n|b\nb" },
  { name: "u", from: "|abc", keys: ["x", "x", "u"], to: "|bc" },
  { name: ".", from: "|a b c", keys: ["d", "w", "."], to: "|c" },
  { name: ">", from: "|a", keys: [">", ">"], to: "  |a" },
  { name: "<", from: "  |a", keys: ["<", "<"], to: "|a" },
  { name: "i", from: "a|bc", keys: ["i"], ins: "X", to: "aX|bc", mode: "insert" },
  { name: "a", from: "a|bc", keys: ["a"], ins: "X", to: "abX|c", mode: "insert" },
  { name: "I", from: "  a|bc", keys: ["I"], ins: "X", to: "  X|abc", mode: "insert" },
  { name: "A", from: "a|bc", keys: ["A"], ins: "X", to: "abcX|", mode: "insert" },
  { name: "o", from: "a|bc\nd", keys: ["o"], ins: "X", to: "abc\nX|\nd", mode: "insert" },
  { name: "O", from: "a|bc\nd", keys: ["O"], ins: "X", to: "X|\nabc\nd", mode: "insert" },
  { name: "v d", from: "|abcd", keys: ["v", "l", "d"], to: "|cd" },
  { name: "V d", from: "a\n|b\nc", keys: ["V", "d"], to: "a\n|c" },
];

/** `|` の位置をカーソルにして、`|` を除いた本文を返す。 */
function at(spec: string): { doc: string; pos: number } {
  return { doc: spec.replace("|", ""), pos: spec.indexOf("|") };
}

test.each(TABLE)("操作仕様書 4.2 の表：$name", ({ from, keys: ks, ins, to, mode }) => {
  const start = at(from);
  const v = make(start.doc);
  v.dispatch({ selection: { anchor: start.pos } });
  keys(v, ...ks);
  if (ins !== undefined) v.dispatch({ changes: { from: v.state.selection.main.head, insert: ins }, selection: { anchor: v.state.selection.main.head + ins.length } });
  const end = at(to);
  expect(v.state.doc.toString()).toBe(end.doc);
  expect(v.state.selection.main.head).toBe(end.pos);
  expect(vimModeOf(v)).toBe(mode ?? "normal");
});

test("操作仕様書 4.2 の表：Ctrl+R のやり直し、検索の n / N、? の逆向き検索", () => {
  const v = make("foo bar\nbaz foo\nfoo");
  keys(v, "x", "u");
  expect(v.state.doc.toString()).toBe("foo bar\nbaz foo\nfoo");
  keys(v, "<C-r>");
  expect(v.state.doc.toString()).toBe("oo bar\nbaz foo\nfoo");
  // ? は後ろ向きに探す
  keys(v, "G", "0");
  keys(v, "?");
  const input = document.querySelector<HTMLInputElement>(".cm-vim-panel input")!;
  input.value = "foo";
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 13, bubbles: true, cancelable: true }));
  // 3 行目の先頭（15）から、後ろ向きに探して 2 行目の foo（11）へ
  expect(v.state.selection.main.head).toBe(11);
});

test("モードの変化を知らせる", () => {
  const modes: VimMode[] = [];
  const v = make("abc", modes);
  keys(v, "i", "<Esc>", "v", "<Esc>");
  expect(modes).toEqual(["normal", "insert", "normal", "visual", "normal"]);
  expect(vimModeOf(v)).toBe("normal");
});

test("Vim の Ctrl のキーは、Mod のキーとして外へ渡す（Vim が使わない）", () => {
  // 標準のキー（Mod+A の全選択など）を足さずに、Vim だけが受けるかを確かめる
  const v = make("abc 12", [], false);
  const before = v.state.doc.toString();
  for (const mode of ["normal", "insert"]) {
    if (mode === "insert") keys(v, "i");
    for (const k of "abcdefghijklmnopqstuvwxy") {
      if (k === "r") continue;
      const r = press(v, { key: k, code: `Key${k.toUpperCase()}`, ctrlKey: true });
      expect(r.consumed, `Ctrl+${k} (${mode})`).toBe(false);
    }
    expect(v.state.doc.toString()).toBe(before);
    expect(vimModeOf(v)).toBe(mode);
  }
});

test("Ctrl を押していても、macOS と AltGr は Vim に渡す", () => {
  const k = (key: string, o: Partial<KeyboardEvent> = {}) => ({ ctrlKey: true, altKey: false, shiftKey: false, key, ...o });
  expect(leavesToApp(k("v"), false)).toBe(true);
  expect(leavesToApp(k("v"), true)).toBe(false);
  expect(leavesToApp(k("@", { altKey: true }), false)).toBe(false);
  expect(leavesToApp(k("R"), false)).toBe(false);
  expect(leavesToApp(k("R", { shiftKey: true }), false)).toBe(true);
  expect(leavesToApp(k("v", { ctrlKey: false }), false)).toBe(false);
});

test("Alt と ⌘ を使うキー（前日・翌日の Alt+[ ]、割当を変えたキー）は、ノーマル・ビジュアルでも Vim が使わず、外へ届く", () => {
  const v = make("abc def", [], false);
  const sample: KeyboardEventInit[] = [
    { key: "[", code: "BracketLeft", altKey: true },
    { key: "]", code: "BracketRight", altKey: true },
    { key: "t", code: "KeyT", altKey: true },
    { key: "T", code: "KeyT", altKey: true, shiftKey: true },
    { key: "ArrowUp", code: "ArrowUp", altKey: true },
    { key: "j", code: "KeyJ", metaKey: true },
    { key: "T", code: "KeyT", metaKey: true, shiftKey: true },
    { key: "Enter", code: "Enter", metaKey: true },
    { key: "Backspace", code: "Backspace", metaKey: true },
  ];
  for (const mode of ["normal", "visual", "insert"]) {
    if (mode === "visual") keys(v, "v");
    if (mode === "insert") keys(v, "<Esc>", "i");
    const before = v.state.doc.toString();
    for (const init of sample) {
      const r = press(v, init);
      expect(r.consumed, `${JSON.stringify(init)} (${mode})`).toBe(false);
    }
    expect(v.state.doc.toString()).toBe(before);
    expect(vimModeOf(v)).toBe(mode);
  }
});

test("挿入モードの日本語入力の変換中の Esc は、変換の取り消しなので挿入モードを抜けない（操作仕様書 3章）", () => {
  const v = make("abc");
  keys(v, "i");
  // WebKit では keyCode 229（isComposing は偽のことがある）、Chromium・Firefox では isComposing が真
  expect(press(v, { key: "Escape", code: "Escape", isComposing: true }).consumed).toBe(false);
  expect(press(v, { key: "Escape", code: "Escape", keyCode: 229 }).consumed).toBe(false);
  expect(vimModeOf(v)).toBe("insert");
  // 変換していなければ抜ける
  press(v, { key: "Escape", code: "Escape" });
  expect(vimModeOf(v)).toBe("normal");
});

test("R（上書き）は INSERT として知らせる。Caps Lock が入っていても Ctrl+R はやり直し", () => {
  const modes: VimMode[] = [];
  const v = make("abc", modes);
  keys(v, "R");
  expect(modes.at(-1)).toBe("insert");
  expect(vimModeOf(v)).toBe("insert");
  keys(v, "<Esc>", "x", "u");
  expect(v.state.doc.toString()).toBe("abc");
  expect(press(v, { key: "R", code: "KeyR", ctrlKey: true }).consumed).toBe(true);
  expect(v.state.doc.toString()).toBe("bc");
});

test("Vim が無効のエディタでも、モードを調べても例外にならない（null）", () => {
  const parent = document.createElement("div");
  document.body.append(parent);
  const v = new EditorView({ parent, state: EditorState.create({ doc: "abc" }) });
  expect(vimModeOf(v)).toBeNull();
  v.destroy();
});

test("Ctrl+Shift+R のように Shift の付いたキーは、続きのキー待ちを捨てずに外へ届く", () => {
  const v = make("a b c");
  keys(v, "d");
  expect(press(v, { key: "R", code: "KeyR", ctrlKey: true, shiftKey: true }).consumed).toBe(false);
  keys(v, "w");
  expect(v.state.doc.toString()).toBe("b c");
});

test("Ctrl+R はやり直し、Ctrl+[ は Esc", () => {
  const v = make("abc");
  keys(v, "x", "u");
  expect(v.state.doc.toString()).toBe("abc");
  expect(press(v, { key: "r", code: "KeyR", ctrlKey: true }).consumed).toBe(true);
  expect(v.state.doc.toString()).toBe("bc");
  keys(v, "i");
  expect(press(v, { key: "[", code: "BracketLeft", ctrlKey: true }).consumed).toBe(true);
  expect(vimModeOf(v)).toBe("normal");
});

test("Mod+Z で取り消せる（Vim のモードのまま）", () => {
  const v = make("abc");
  keys(v, "x");
  expect(v.state.doc.toString()).toBe("bc");
  press(v, { key: "z", code: "KeyZ", ctrlKey: true });
  expect(v.state.doc.toString()).toBe("abc");
});

test("<Space>t <Space>b <Space>h は、ノートの操作を呼ぶ", async () => {
  const calls: string[] = [];
  const host: VimHost = {
    send: async (_v, kind) => void calls.push(`${kind}:${v.state.selection.main.from}-${v.state.selection.main.to}`),
    timeHeading: (vv) => {
      calls.push("heading");
      vv.dispatch({ changes: { from: 0, insert: "## " }, selection: { anchor: 3 } });
      return true;
    },
  };
  const v = make("one\ntwo\nthree");
  setVimHost(v, host);
  // jsdom には画面の配置が無く、j / k（見た目の行での移動）は正しく動かないので、位置は直接決める
  v.dispatch({ selection: { anchor: 4 } });
  keys(v, "<Space>t");
  keys(v, "<Space>b");
  expect(calls).toEqual(["todo:4-4", "blocker:4-4"]);
  keys(v, "<Space>h");
  expect(calls.at(-1)).toBe("heading");
  expect(vimModeOf(v)).toBe("insert");
  // 挿入モードの入力は、ブラウザが入れる（Vim のキーの処理は通さない）ので、直接入れる
  v.dispatch({ changes: { from: v.state.selection.main.head, insert: "x" } });
  keys(v, "<Esc>");
  expect(v.state.doc.toString()).toBe("## onex\ntwo\nthree");
  // ビジュアル（行）：選択した行を送り、送り終えたらノーマルに戻る
  keys(v, "gg", "V", "G");
  keys(v, "<Space>t");
  await new Promise((r) => setTimeout(r, 0));
  expect(calls.at(-1)).toMatch(/^todo:0-/);
  expect(vimModeOf(v)).toBe("normal");
});

test("/ と ? は、下の帯を使わずに検索欄を開き、Enter で一致した所へ移る。n / N で次・前へ", () => {
  const v = make("foo bar\nbaz foo\nfoo");
  keys(v, "/");
  const input = document.querySelector<HTMLInputElement>(".cm-vim-panel input")!;
  expect(input).not.toBeNull();
  input.value = "foo";
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 13, bubbles: true, cancelable: true }));
  expect(document.querySelector(".cm-vim-panel input")).toBeNull();
  const heads = [v.state.selection.main.head];
  keys(v, "n");
  heads.push(v.state.selection.main.head);
  keys(v, "N");
  heads.push(v.state.selection.main.head);
  expect(heads).toEqual([12, 16, 12]);
});

test("ノーマルモードでは、Vim に無いキーも、続きのキー待ちの後の Space も文字の入力にならない", () => {
  const v = make("abc");
  const consumed = (key: string, code: string) => press(v, { key, code }).consumed;
  // 単独のキー、続きのキー待ち（d g z や <Space>）の後の Space、<Space> の後の別のキー
  for (const [a, b] of [["Q", "Q"], ["d", " "], ["g", " "], ["z", " "], [" ", " "], [" ", "z"]]) {
    expect([a, b, consumed(a, `Key${a.toUpperCase()}`), consumed(b, b === " " ? "Space" : `Key${b.toUpperCase()}`)]).toEqual([a, b, true, true]);
    press(v, { key: "Escape", code: "Escape" });
  }
  expect(v.state.doc.toString()).toBe("abc");
  // 挿入モードの Space は文字なので、Vim も止めない
  keys(v, "i");
  expect(consumed(" ", "Space")).toBe(false);
});

test("<Space> の後の別のキーは捨てて、その次のキーから受ける", () => {
  const v = make("abc");
  const heads: number[] = [];
  for (const [key, code] of [[" ", "Space"], ["z", "KeyZ"], ["l", "KeyL"]]) {
    press(v, { key, code });
    heads.push(v.state.selection.main.head);
  }
  expect(heads).toEqual([0, 0, 1]);
});

test("振り返り（ホストなし）では <Space>t <Space>b <Space>h は何も起こさない", () => {
  const v = make("one\ntwo");
  keys(v, "<Space>t", "<Space>b", "<Space>h");
  expect(v.state.doc.toString()).toBe("one\ntwo");
  expect(v.state.selection.main.head).toBe(0);
  expect(vimModeOf(v)).toBe("normal");
});
