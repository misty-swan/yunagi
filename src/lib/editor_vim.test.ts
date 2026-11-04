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
  expect(leavesToApp({ ctrlKey: true, altKey: false, key: "v" }, false)).toBe(true);
  expect(leavesToApp({ ctrlKey: true, altKey: false, key: "v" }, true)).toBe(false);
  expect(leavesToApp({ ctrlKey: true, altKey: true, key: "@" }, false)).toBe(false);
  expect(leavesToApp({ ctrlKey: true, altKey: false, key: "R" }, false)).toBe(false);
  expect(leavesToApp({ ctrlKey: false, altKey: false, key: "v" }, false)).toBe(false);
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
