import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import type { EditorView } from "@codemirror/view";
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
}

function mockCore(body: string): Core {
  const c: Core = { calls: [], note: { body, rev: body === "" ? 0 : 1 } };
  mockIPC((cmd, args) => {
    const a = args as Record<string, unknown>;
    c.calls.push({ cmd, args: a });
    switch (cmd) {
      case "get_text_doc":
        return a.doc === "note" ? { ...c.note } : { body: "", rev: 0 };
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
  props: { refresh: number; visible: boolean };
}

async function start(body: string, opts: { font?: FontLoader; visible?: boolean } = {}): Promise<Started> {
  const core = mockCore(body);
  const save = new SaveStatus();
  tracker = new InputTracker(save, undefined, true);
  const props = $state({ refresh: 0, visible: opts.visible ?? true });
  let view: EditorView | null = null;
  app = mount(NoteView, {
    target: document.body,
    props: {
      date: TODAY,
      save,
      presence: tracker.note,
      get refresh() {
        return props.refresh;
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
  props.refresh++;
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
