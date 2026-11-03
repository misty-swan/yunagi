import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { EditorView } from "@codemirror/view";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { SaveStatus } from "../lib/autosave.svelte";
import { FONT_WAIT_MS, FontLoader } from "../lib/fonts.svelte";
import { prefs } from "../lib/prefs.svelte";
import { InputTracker } from "../lib/ui_report.svelte";
import NoteView from "./NoteView.svelte";

const TODAY = "2026-11-03";

// jsdom には Range の位置が無い（CodeMirror が測るときに使う）
Range.prototype.getClientRects ??= () => [] as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();

let app: ReturnType<typeof mount> | undefined;
let tracker: InputTracker | undefined;

beforeEach(() => {
  prefs.lang = "ja";
  prefs.noteFont = "body";
});

afterEach(async () => {
  if (app) unmount(app);
  app = undefined;
  tracker?.stop();
  document.body.innerHTML = "";
  vi.useRealTimers();
  await new Promise((r) => setTimeout(r, 0));
  clearMocks();
});

interface Core {
  calls: { cmd: string; args: Record<string, unknown> }[];
  note: { body: string; rev: number };
  /** あれば、ノートを読む応答をこれが解決するまで待たせる。 */
  gate: Promise<void> | null;
}

function mockCore(body: string): Core {
  const c: Core = { calls: [], note: { body, rev: body === "" ? 0 : 1 }, gate: null };
  mockIPC(async (cmd, args) => {
    const a = args as Record<string, unknown>;
    c.calls.push({ cmd, args: a });
    switch (cmd) {
      case "get_text_doc": {
        if (a.doc !== "note") return { body: "", rev: 0 };
        const snap = { ...c.note };
        if (c.gate !== null) await c.gate;
        return snap;
      }
      case "save_text_doc":
        if (a.baseRev !== c.note.rev) return { result: "stale", latest: { ...c.note } };
        c.note = { body: a.body as string, rev: c.note.rev + 1 };
        return { result: "saved", rev: c.note.rev, updated: true };
      case "report_ui_state":
        return null;
    }
    return null;
  });
  return c;
}

async function until(check: () => boolean) {
  await vi.waitFor(() => {
    flushSync();
    expect(check()).toBe(true);
  });
}

const q = (sel: string) => document.querySelector<HTMLElement>(sel);
const qa = (sel: string) => [...document.querySelectorAll<HTMLElement>(sel)];
const text = (el: Element | null) => el?.textContent?.replace(/\s+/g, " ").trim() ?? "";

interface Started {
  core: Core;
  view: () => EditorView;
  props: { rev: number; visible: boolean; date: string };
}

async function start(body: string, opts: { font?: FontLoader; visible?: boolean } = {}): Promise<Started> {
  const core = mockCore(body);
  const save = new SaveStatus();
  tracker = new InputTracker(save, undefined, true);
  const props = $state({ rev: core.note.rev, visible: opts.visible ?? true, date: TODAY });
  let view: EditorView | null = null;
  app = mount(NoteView, {
    target: document.body,
    props: {
      get date() {
        return props.date;
      },
      save,
      presence: tracker.note,
      get rev() {
        return props.rev;
      },
      onActivity: tracker.activity,
      get visible() {
        return props.visible;
      },
      narrow: false,
      mac: false,
      font: opts.font ?? new FontLoader(async () => true),
      now: () => new Date(2026, 10, 3, 14, 5),
      get view() {
        return view;
      },
      set view(v) {
        view = v;
      },
    },
  });
  await until(() => view !== null && view.state.doc.toString() === body && tracker!.note.saver !== null);
  return { core, view: () => view!, props };
}

function press(init: KeyboardEventInit, target: EventTarget = window) {
  target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  flushSync();
}

const H = { key: "H", code: "KeyH", keyCode: 72, ctrlKey: true, shiftKey: true };

test("その日のノートを出し、時刻付き見出しを時刻と件名に分けて飾り、Outline に並べる", async () => {
  const { view } = await start("前置き\n## 10:00 定例\n- 議題\n## 良かったこと\n## 13:30\nメモ");
  const lines = qa(".note-ed .cm-line");
  expect(lines.map((l) => l.classList.contains("cm-th"))).toEqual([false, true, false, false, true, false]);
  expect(text(lines[1].querySelector(".cm-md"))).toBe("##");
  expect(text(lines[1].querySelector(".cm-tm"))).toBe("10:00");
  // 時刻の無い見出しは、書いたとおりの本文
  expect(lines[3].querySelector(".cm-md")).toBeNull();
  expect(qa(".outline button").map(text)).toEqual(["10:00 定例", "13:30"]);
  // カーソルのある区間の丸を塗り、Outline でも示す（はじめは先頭の前置きなので、どれでもない）
  expect(qa(".cm-th-cur")).toEqual([]);
  view().dispatch({ selection: { anchor: view().state.doc.line(3).from } });
  flushSync();
  expect(qa(".note-ed .cm-line")[1].classList.contains("cm-th-cur")).toBe(true);
  expect(text(q(".outline button.cur"))).toBe("10:00 定例");
  // 本文の行を打っても、見出しが変わらなければ Outline は描き直さない
  const first = qa(".outline button")[0];
  view().dispatch({ changes: { from: view().state.doc.line(3).to, insert: "追加" } });
  flushSync();
  expect(qa(".outline button")[0]).toBe(first);
  // 見出しの行を書き換えると、一覧も変わる
  view().dispatch({ changes: { from: view().state.doc.line(2).to, insert: "会" } });
  flushSync();
  expect(qa(".outline button").map(text)).toEqual(["10:00 定例会", "13:30"]);
  // Outline の見出しを選ぶと、その見出しの行の終わりへ移る
  qa(".outline button")[1].click();
  flushSync();
  expect(view().state.selection.main.head).toBe(view().state.doc.line(5).to);
  expect(text(q(".outline button.cur"))).toBe("13:30");
  expect(q(".outline button.cur")?.getAttribute("aria-current")).toBe("location");
});

test("Mod+Shift+H で、現在時刻の見出しを入れて保存し、エディタの外でも受ける。ノートモードでなければ受けない", async () => {
  const { core, view, props } = await start("メモ");
  view().dispatch({ selection: { anchor: 2 } });
  press(H);
  expect(view().state.doc.toString()).toBe("メモ\n\n## 14:05 ");
  expect(view().state.selection.main.head).toBe(view().state.doc.length);
  expect(view().hasFocus).toBe(true);
  await until(() => core.note.body === "メモ\n\n## 14:05 ");
  expect(qa(".outline button").map(text)).toEqual(["14:05"]);
  // 空の行ならその行に入れる
  view().dispatch({ changes: { from: view().state.doc.length, insert: "定例\n" }, selection: { anchor: view().state.doc.length + 3 } });
  press(H);
  expect(view().state.doc.toString()).toBe("メモ\n\n## 14:05 定例\n## 14:05 ");
  props.visible = false;
  flushSync();
  press(H);
  expect(view().state.doc.toString()).toBe("メモ\n\n## 14:05 定例\n## 14:05 ");
});

test("「- 」の行の Enter で次の行も「- 」で始め、中身のない「- 」では「- 」を消す", async () => {
  const { view } = await start("- 買う");
  const content = q(".note-ed .cm-content")!;
  content.focus();
  view().dispatch({ selection: { anchor: 4 } });
  press({ key: "Enter", code: "Enter", keyCode: 13 }, content);
  expect(view().state.doc.toString()).toBe("- 買う\n- ");
  press({ key: "Enter", code: "Enter", keyCode: 13 }, content);
  expect(view().state.doc.toString()).toBe("- 買う\n");
});

test("エージェントの追記を取り込んでも、入力中の行とカーソルを保ち、守る行を知らせる", async () => {
  const { core, view, props } = await start("a\nb");
  const content = q(".note-ed .cm-content")!;
  content.focus();
  // 2行目を書き換えている途中（まだ保存していない）
  view().dispatch({ changes: { from: 3, insert: "XY" }, selection: { anchor: 4 }, userEvent: "input" });
  await until(() => core.note.body === "a\nbXY");
  view().dispatch({ changes: { from: 5, insert: "Z" }, selection: { anchor: 6 }, userEvent: "input" });
  flushSync();
  // 守る行（カーソル行の 2行目）を、最後に保存した版で知らせる
  await until(() =>
    core.calls.some(
      (c) =>
        c.cmd === "report_ui_state" &&
        JSON.stringify((c.args.state as { editors: unknown[] }).editors) === JSON.stringify([{ doc: "note", date: TODAY, rev: 2, lines: [2] }]),
    ),
  );
  // エージェントが末尾に追記した
  core.note = { body: "a\nbXY\n## 15:00 追記", rev: 3 };
  // 版が進んでいなければ読み直さない
  const reads = core.calls.filter((c) => c.cmd === "get_text_doc").length;
  props.rev = 2;
  flushSync();
  expect(core.calls.filter((c) => c.cmd === "get_text_doc").length).toBe(reads);
  props.rev = 3;
  await until(() => view().state.doc.toString() === "a\nbXYZ\n## 15:00 追記");
  expect(view().state.selection.main.head).toBe(6);
  await until(() => core.note.body === "a\nbXYZ\n## 15:00 追記");
  // 取り込んだ変更は取り消しの対象にしない（Mod+Z で消えるのは利用者の入力だけ）
  expect(qa(".outline button").map(text)).toEqual(["15:00 追記"]);
});

test("等幅：読み込み終えてから当て、起動時は読み込み終えるまで文字を出さない。読めなければ本文の書体", async () => {
  let resolve!: (ok: boolean) => void;
  const font = new FontLoader(() => new Promise<boolean>((r) => (resolve = r)));
  prefs.noteFont = "mono";
  await start("a", { font });
  expect(q(".note")?.dataset.font).toBe("body");
  expect(q(".note")?.hasAttribute("data-ready")).toBe(false);
  resolve(true);
  await until(() => q(".note")?.dataset.font === "mono");
  expect(q(".note")?.hasAttribute("data-ready")).toBe(true);
  // 本文と同じに戻す・もう一度等幅にする（読み込み済みなのですぐ当てる）
  prefs.noteFont = "body";
  flushSync();
  expect(q(".note")?.dataset.font).toBe("body");
  prefs.noteFont = "mono";
  flushSync();
  expect(q(".note")?.dataset.font).toBe("mono");
});

test("等幅を読み込めないときは本文の書体のまま出し、待ちの上限を過ぎたら本文の書体で描く", async () => {
  prefs.noteFont = "mono";
  await start("a", { font: new FontLoader(async () => false) });
  await until(() => q(".note")?.hasAttribute("data-ready") === true);
  expect(q(".note")?.dataset.font).toBe("body");
  unmount(app!);
  app = undefined;
  tracker?.stop();
  document.body.innerHTML = "";

  vi.useFakeTimers({ shouldAdvanceTime: true });
  let resolve!: (ok: boolean) => void;
  await start("a", { font: new FontLoader(() => new Promise<boolean>((r) => (resolve = r))) });
  expect(q(".note")?.hasAttribute("data-ready")).toBe(false);
  await vi.advanceTimersByTimeAsync(FONT_WAIT_MS);
  flushSync();
  expect(q(".note")?.hasAttribute("data-ready")).toBe(true);
  expect(q(".note")?.dataset.font).toBe("body");
  // 後から読み込み終えたら、そのとき等幅に切り替える
  resolve(true);
  await until(() => q(".note")?.dataset.font === "mono");
});

test("見出しの挿入は1回の取り消しで戻り、続けて打った件名とまとめない。押し続けても重ねて入れない", async () => {
  const { view } = await start("");
  const content = q(".note-ed .cm-content")!;
  content.focus();
  press(H);
  view().dispatch({ changes: { from: view().state.doc.length, insert: "定例" }, userEvent: "input.type" });
  press({ ...H, repeat: true });
  expect(view().state.doc.toString()).toBe("## 14:05 定例");
  press({ key: "z", code: "KeyZ", keyCode: 90, ctrlKey: true }, content);
  expect(view().state.doc.toString()).toBe("## 14:05 ");
  press({ key: "z", code: "KeyZ", keyCode: 90, ctrlKey: true }, content);
  expect(view().state.doc.toString()).toBe("");
});

test("ほかの入力欄にフォーカスがあるときは、Mod+Shift+H を受けない", async () => {
  const { view } = await start("");
  const input = document.createElement("input");
  document.body.append(input);
  input.focus();
  press(H, input);
  expect(view().state.doc.toString()).toBe("");
  expect(document.activeElement).toBe(input);
});

test("日を移ってすぐ戻っても、古い読み込みが入力を消さない。読んでいる間に進んだ版は読み終えてから取り込む", async () => {
  const { core, view, props } = await start("a");
  let open!: () => void;
  core.gate = new Promise<void>((r) => (open = r));
  props.date = "2026-11-04";
  flushSync();
  props.date = TODAY;
  flushSync();
  // 読んでいる間に、エージェントが書いた（ボードの版が進んだ）
  core.note = { body: "a\nb", rev: 2 };
  props.rev = 2;
  flushSync();
  core.gate = null;
  open();
  await until(() => view().state.doc.toString() === "a\nb" && view().state.facet(EditorView.editable));
  expect(core.note.body).toBe("a\nb");
});

test("言語を変えると、エディタの読み上げ用の名前を付け直す", async () => {
  await start("a");
  expect(q(".note-ed .cm-content")?.getAttribute("aria-label")).toBe("ノート");
  prefs.lang = "en";
  flushSync();
  expect(q(".note-ed .cm-content")?.getAttribute("aria-label")).toBe("Note");
});

test("CR は行の区切りにせず、そのまま残す（コアと同じ数え方で、保存で書き換えない）", async () => {
  const { core, view } = await start("## 10:00 定例\r\n一\r");
  expect(view().state.doc.lines).toBe(2);
  view().dispatch({ changes: { from: view().state.doc.length, insert: "二" }, userEvent: "input" });
  await until(() => core.note.body === "## 10:00 定例\r\n一\r二");
});

test("1回の書き換えで本文の行を見出しの上から下へ動かしても、見出しの位置を読み直す", async () => {
  const { view } = await start("a\nb\n## 10:00 定例\nc");
  // 1行目を消して、最後に足す（行の数は同じで、見出しは 3行目から 2行目へ）
  view().dispatch({ changes: [{ from: 0, to: 2 }, { from: view().state.doc.length, insert: "\na" }] });
  flushSync();
  expect(qa(".note-ed .cm-line").map((l) => l.classList.contains("cm-th"))).toEqual([false, true, false, false]);
  qa(".outline button")[0].click();
  expect(view().state.selection.main.head).toBe(view().state.doc.line(2).to);
});

test("貼り付けた文の改行（CRLF・CR）は LF にそろえる", async () => {
  const { core, view } = await start("");
  const content = q(".note-ed .cm-content")!;
  content.focus();
  // jsdom には DataTransfer が無いので、CodeMirror が読む getData だけを持つものを渡す
  const data = { getData: (t: string) => (t === "text/plain" ? "一\r\n二\r三" : ""), types: ["text/plain"] };
  content.dispatchEvent(Object.assign(new Event("paste", { bubbles: true, cancelable: true }), { clipboardData: data }));
  flushSync();
  expect(view().state.doc.toString()).toBe("一\n二\n三");
  await until(() => core.note.body === "一\n二\n三");
});
