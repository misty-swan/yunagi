import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { EditorView } from "@codemirror/view";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { SaveStatus } from "../lib/autosave.svelte";
import type { ItemKind, NoteSource } from "../lib/bindings";
import { FONT_WAIT_MS, FontLoader } from "../lib/fonts.svelte";
import { Notice } from "../lib/notice.svelte";
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
  /** 送った行（模擬の照合：本文が同じ行に印を付ける）。 */
  sends: { text: string; kind: ItemKind; itemId: string }[];
  /** 真なら、ノートを読む・行を送る・印を求めるのに失敗する。 */
  fail: { read?: boolean; save?: boolean; send?: boolean; marks?: boolean };
  /** あれば、次の印の問い合わせの答えを、これが解決するまで待たせる。 */
  marksGate: Promise<void> | null;
}

function mockCore(body: string): Core {
  const c: Core = { calls: [], note: { body, rev: body === "" ? 0 : 1 }, gate: null, sends: [], fail: {}, marksGate: null };
  mockIPC(async (cmd, args) => {
    const a = args as Record<string, unknown>;
    c.calls.push({ cmd, args: a });
    switch (cmd) {
      case "get_text_doc": {
        if (a.doc !== "note") return { body: "", rev: 0 };
        if (c.fail.read) throw { kind: "storage", detail: "x" };
        const snap = { ...c.note };
        if (c.gate !== null) await c.gate;
        return snap;
      }
      case "save_text_doc":
        if (c.fail.save) throw { kind: "storage", detail: "x" };
        if (a.baseRev !== c.note.rev) return { result: "stale", latest: { ...c.note } };
        c.note = { body: a.body as string, rev: c.note.rev + 1 };
        return { result: "saved", rev: c.note.rev, updated: true };
      case "report_ui_state":
        return null;
      case "send_note_lines": {
        if (c.fail.send) throw { kind: "storage", detail: "x" };
        if (a.baseRev !== c.note.rev) return { result: "stale", body: c.note.body, rev: c.note.rev };
        const lines = (a.body as string).split("\n");
        if (a.body !== c.note.body) c.note = { body: a.body as string, rev: c.note.rev + 1 };
        const sent = [];
        const skipped = [];
        for (let n = a.from as number; n <= (a.to as number); n++) {
          const t = lines[n - 1];
          if (c.sends.some((x) => x.text === t)) skipped.push({ line: n, reason: "already_sent" });
          else if (t.trim() === "" || t.startsWith("#")) skipped.push({ line: n, reason: t.startsWith("#") ? "heading" : "no_text" });
          else {
            const itemId = `item-${c.sends.length + 1}`;
            c.sends.push({ text: t, kind: a.kind as ItemKind, itemId });
            sent.push({ line: n, item: { id: itemId, text: t.replace(/^- /, "") } });
          }
        }
        return { result: "sent", rev: c.note.rev, sent, skipped };
      }
      case "note_marks": {
        if (c.fail.marks) throw { kind: "storage", detail: "x" };
        const out = [];
        for (const [i, t] of (a.body as string).split("\n").entries()) {
          for (const x of c.sends) if (x.text === t) out.push({ line: i + 1, kind: x.kind, itemId: x.itemId });
        }
        // 照合した後に、答えるのを待たせる（1回だけ）
        const gate = c.marksGate;
        c.marksGate = null;
        if (gate !== null) await gate;
        return out;
      }
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
  props: {
    rev: number;
    visible: boolean;
    date: string;
    items: string;
    itemsDate: string | null;
    jump: NoteSource | null;
    narrow: boolean;
  };
  notice: Notice;
}

async function start(body: string, opts: { font?: FontLoader; visible?: boolean } = {}): Promise<Started> {
  const core = mockCore(body);
  const save = new SaveStatus();
  const notice = new Notice();
  tracker = new InputTracker(save, undefined, true);
  const props = $state({
    rev: core.note.rev,
    visible: opts.visible ?? true,
    date: TODAY,
    items: "",
    itemsDate: TODAY as string | null,
    jump: null as NoteSource | null,
    narrow: false,
  });
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
      get narrow() {
        return props.narrow;
      },
      mac: false,
      notice,
      get items() {
        return props.items;
      },
      get itemsDate() {
        return props.itemsDate;
      },
      get jump() {
        return props.jump;
      },
      onJumped: () => (props.jump = null),
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
  return { core, view: () => view!, props, notice };
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

const T = { key: "T", code: "KeyT", keyCode: 84, ctrlKey: true, shiftKey: true };
const B = { key: "B", code: "KeyB", keyCode: 66, ctrlKey: true, shiftKey: true };
const sentLabels = () => qa(".note-ed .cm-line").map((l) => l.dataset.sent ?? "");
const sentCounts = () => qa(".sent .counts b").map(text);

test("Mod+Shift+T で現在行を TODO へ送り、本文の保存を兼ねる。送った行に印を付け、Sent に行数を出す", async () => {
  const { core, view } = await start("## 10:00 定例\n- 調べる\n- 待つ");
  const content = q(".note-ed .cm-content")!;
  content.focus();
  // まだ保存していない入力も、送るときに一緒に保存する
  view().dispatch({ changes: { from: view().state.doc.line(2).to, insert: "こと" }, selection: { anchor: view().state.doc.line(2).to + 2 } });
  press(T);
  await until(() => sentLabels()[1] === "→ TODO");
  const call = core.calls.find((c) => c.cmd === "send_note_lines")!;
  expect(call.args).toMatchObject({ date: TODAY, body: "## 10:00 定例\n- 調べること\n- 待つ", baseRev: 1, from: 2, to: 2, kind: "todo" });
  expect(core.note).toEqual({ body: "## 10:00 定例\n- 調べること\n- 待つ", rev: 2 });
  expect(sentLabels()).toEqual(["", "→ TODO", ""]);
  expect(sentCounts()).toEqual(["1", "0"]);
  // 送った後の保存は、送った後の版をもとにする
  view().dispatch({ changes: { from: view().state.doc.length, insert: "!" }, userEvent: "input" });
  await until(() => core.note.body.endsWith("待つ!"));
  expect(core.note.rev).toBe(3);
  // 行を上に足しても、印は行に付いたまま動く（求め直す前から）
  view().dispatch({ changes: { from: 0, insert: "前\n" } });
  flushSync();
  expect(sentLabels()).toEqual(["", "", "→ TODO", ""]);
  // 送った行の本文を変えると、求め直したときに印が外れる
  view().dispatch({ changes: { from: view().state.doc.line(3).to, insert: "?" } });
  await until(() => sentLabels().every((l) => l === ""));
  expect(sentCounts()).toEqual(["0", "0"]);
});

test("選んだ複数行を Mod+Shift+B で障害へ送る。送る操作は取り消しの区切りにする", async () => {
  const { core, view } = await start("- a\n- b\n- c");
  const content = q(".note-ed .cm-content")!;
  content.focus();
  view().dispatch({ changes: { from: view().state.doc.length, insert: "c" }, userEvent: "input.type" });
  // 1行目から3行目の先頭まで（3行目は含めない）
  view().dispatch({ selection: { anchor: 0, head: view().state.doc.line(3).from } });
  press(B);
  await until(() => sentLabels().join("|") === "→ BLOCKER|→ BLOCKER|");
  expect(core.calls.find((c) => c.cmd === "send_note_lines")!.args).toMatchObject({ from: 1, to: 2, kind: "blocker" });
  expect(sentCounts()).toEqual(["0", "2"]);
  // 送った後に打った文字だけが、1回の取り消しで戻る
  view().dispatch({ changes: { from: view().state.doc.length, insert: "d" }, userEvent: "input.type" });
  press({ key: "z", code: "KeyZ", keyCode: 90, ctrlKey: true }, content);
  expect(view().state.doc.toString()).toBe("- a\n- b\n- cc");
});

test("送れる行が無いとき・送信済みのとき・送れなかったときは、上部のバーに短く知らせる", async () => {
  const { core, view, notice } = await start("## 10:00 定例\n- 調べる");
  q(".note-ed .cm-content")!.focus();
  view().dispatch({ selection: { anchor: 0 } });
  press(T);
  await until(() => notice.text === "送れる行がありません");
  view().dispatch({ selection: { anchor: view().state.doc.length } });
  press(T);
  await until(() => sentLabels()[1] === "→ TODO");
  press(B);
  await until(() => notice.text === "送信済みの行です");
  core.fail.send = true;
  view().dispatch({ changes: { from: view().state.doc.length, insert: "\n- 新しい" }, selection: { anchor: view().state.doc.length + 6 } });
  press(T);
  await until(() => notice.text === "行を送れませんでした");
  // 送れなくても、本文はふだんの自動保存で保存する
  await until(() => core.note.body.endsWith("- 新しい"));
});

test("行送りの版が古ければ、エージェントの変更を取り込んでから、ずれた行で送り直す", async () => {
  const { core, view } = await start("- 調べる");
  q(".note-ed .cm-content")!.focus();
  view().dispatch({ selection: { anchor: view().state.doc.length } });
  // エージェントが上に1行足していた（画面はまだ知らない）
  core.note = { body: "## 09:00 朝会\n- 調べる", rev: 2 };
  press(T);
  await until(() => view().state.doc.toString() === "## 09:00 朝会\n- 調べる" && sentLabels()[1] === "→ TODO");
  const sends = core.calls.filter((c) => c.cmd === "send_note_lines").map((c) => c.args);
  expect(sends.map((a) => [a.baseRev, a.from])).toEqual([
    [1, 1],
    [2, 2],
  ]);
});

test("項目の「← NOTE」から移ると、その日の送信済みの印の行（無ければ保存済みの本文の行）にカーソルを置く", async () => {
  const { core, view, props } = await start("前\n- 送った");
  core.sends.push({ text: "- 送った", kind: "todo", itemId: "item-x" });
  props.items = "item-x";
  await until(() => sentLabels()[1] === "→ TODO");
  // 保存していない行を上に足した（保存済みの本文では 2行目）
  view().dispatch({ changes: { from: 0, insert: "足した\n" } });
  await until(() => sentLabels()[2] === "→ TODO");
  props.jump = { date: TODAY, line: 2, itemId: "item-x" };
  await until(() => props.jump === null);
  expect(view().state.selection.main.head).toBe(view().state.doc.line(3).to);
  expect(view().hasFocus).toBe(true);
  // 印の無い項目（送り先を削除した元の項目など）は、保存済みの本文の行へ
  props.jump = { date: TODAY, line: 1, itemId: "item-gone" };
  await until(() => props.jump === null);
  expect(view().state.selection.main.head).toBe(view().state.doc.line(1).to);
  // 別の日へ移る途中なら、その日を読み終えるまで待つ
  props.jump = { date: "2026-11-02", line: 1, itemId: "item-x" };
  flushSync();
  expect(props.jump).not.toBeNull();
  props.date = "2026-11-02";
  await until(() => props.jump === null);
});

test("Sent は Outline の下に出し、900px 未満では Outline とともに出さない", async () => {
  const { props } = await start("a");
  expect(q(".note-side .outline + .sent")).not.toBeNull();
  expect(qa(".sent .counts span").map(text)).toEqual(["To TODO 0", "To Blockers 0"]);
  props.narrow = true;
  flushSync();
  expect(q(".note")?.hasAttribute("data-narrow")).toBe(true);
});

test("版が進んだときの読み直しに失敗したら、版が変わらなくても読み直す", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  const { core, view, props } = await start("a");
  core.note = { body: "a\nb", rev: 2 };
  core.fail.read = true;
  props.rev = 2;
  flushSync();
  await vi.advanceTimersByTimeAsync(10);
  expect(view().state.doc.toString()).toBe("a");
  core.fail.read = false;
  await vi.advanceTimersByTimeAsync(1000);
  await until(() => view().state.doc.toString() === "a\nb");
  // 取り込んだら、それ以上は読み直さない
  const reads = core.calls.filter((c) => c.cmd === "get_text_doc").length;
  await vi.advanceTimersByTimeAsync(3000);
  expect(core.calls.filter((c) => c.cmd === "get_text_doc").length).toBe(reads);
});

test("CR は印にして見せ、エディタの中で写して貼り付けた文の CR は保つ", async () => {
  const { core, view } = await start("一\r二");
  expect(qa(".note-ed .cm-specialChar").length).toBe(1);
  const content = q(".note-ed .cm-content")!;
  content.focus();
  // エディタから写す（CodeMirror が clipboardData に書く文を受け取る）
  view().dispatch({ selection: { anchor: 0, head: view().state.doc.length } });
  let copied = "";
  const out = { setData: (_t: string, v: string) => (copied = v), clearData: () => {}, getData: () => "", types: [] };
  content.dispatchEvent(Object.assign(new Event("copy", { bubbles: true, cancelable: true }), { clipboardData: out }));
  expect(copied).toBe("一\r二");
  view().dispatch({ selection: { anchor: view().state.doc.length } });
  const data = { getData: (t: string) => (t === "text/plain" ? copied : ""), types: ["text/plain"] };
  content.dispatchEvent(Object.assign(new Event("paste", { bubbles: true, cancelable: true }), { clipboardData: data }));
  flushSync();
  expect(view().state.doc.toString()).toBe("一\r二一\r二");
  await until(() => core.note.body === "一\r二一\r二");
});

test("複数のカーソルで本文の行を打っても、見出しの一覧を読み直さない", async () => {
  const { view } = await start("a\n## 10:00 定例\nb");
  const first = qa(".outline button")[0];
  view().dispatch({
    changes: [
      { from: 1, insert: "x" },
      { from: view().state.doc.length, insert: "y" },
    ],
  });
  flushSync();
  expect(qa(".outline button")[0]).toBe(first);
  // 1か所で行を足し、別の所で行を消すと（全体の行の数は同じでも）、見出しの位置を読み直す
  view().dispatch({
    changes: [
      { from: 0, insert: "上\n" },
      { from: view().state.doc.line(3).from - 1, to: view().state.doc.length },
    ],
  });
  flushSync();
  expect(view().state.doc.toString()).toBe("上\nax\n## 10:00 定例");
  expect(qa(".note-ed .cm-line").map((l) => l.classList.contains("cm-th"))).toEqual([false, false, true]);
});

test("行の右クリックのメニューから TODO・障害へ送り、Esc で閉じる", async () => {
  const { core, view } = await start("- a\n- b");
  const content = q(".note-ed .cm-content")!;
  content.focus();
  // jsdom では位置を測れないので、押した所を 2行目にする（カーソルはその位置へ移る）
  view().posAtCoords = (() => 5) as EditorView["posAtCoords"];
  const right = () =>
    content.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 5, clientY: 5 }));
  right();
  await until(() => q(".line-menu") !== null && document.activeElement === qa(".line-menu button")[0]);
  expect(qa(".line-menu button").map(text)).toEqual(["TODO へ送るCtrl+Shift+T", "障害へ送るCtrl+Shift+B"]);
  press({ key: "ArrowDown" }, document.activeElement!);
  expect(document.activeElement).toBe(qa(".line-menu button")[1]);
  qa(".line-menu button")[1].click();
  await until(() => sentLabels()[1] === "→ BLOCKER");
  expect(q(".line-menu")).toBeNull();
  expect(core.calls.find((c) => c.cmd === "send_note_lines")!.args).toMatchObject({ from: 2, to: 2, kind: "blocker" });
  right();
  await until(() => q(".line-menu") !== null);
  press({ key: "Escape" }, document.activeElement!);
  expect(q(".line-menu")).toBeNull();
  expect(view().hasFocus).toBe(true);
});

test("送信済みの印のある行の先頭で Enter を押すと、印は下へ動いた本文に付いていく", async () => {
  const { core, view, props } = await start("前\n- 送った");
  core.sends.push({ text: "- 送った", kind: "todo", itemId: "item-x" });
  props.items = "item-x";
  await until(() => sentLabels()[1] === "→ TODO");
  // 求め直さずに、動かすだけの結果を見る
  core.fail.marks = true;
  view().dispatch({ changes: { from: view().state.doc.line(2).from, insert: "\n" } });
  flushSync();
  expect(sentLabels()).toEqual(["", "", "→ TODO"]);
});

test("その日のノートから送った項目が無ければ、印をコアに問い合わせない。古い問い合わせの答えは使わない", async () => {
  const { core, view, props } = await start("- a");
  await new Promise((r) => setTimeout(r, 50));
  view().dispatch({ changes: { from: 1, insert: "b" } });
  await new Promise((r) => setTimeout(r, 250));
  expect(core.calls.filter((c) => c.cmd === "note_marks")).toEqual([]);
  // 先に始めた問い合わせ（まだ送った記録が無い）が、後から始めたものより後に答えても、後のものを使う
  let release!: () => void;
  core.marksGate = new Promise<void>((r) => (release = r));
  props.items = "item-x";
  await until(() => core.calls.some((c) => c.cmd === "note_marks"));
  core.sends.push({ text: view().state.doc.line(1).text, kind: "todo", itemId: "item-x" });
  props.items = "item-x,item-y";
  await until(() => sentLabels()[0] === "→ TODO");
  release();
  await new Promise((r) => setTimeout(r, 20));
  expect(sentLabels()[0]).toBe("→ TODO");
});

test("時刻付き見出しの行を同じ本文で書き換えても、時刻の飾りを残す", async () => {
  const { view } = await start("## 10:00 定例\nメモ");
  const line = view().state.doc.line(1);
  view().dispatch({ changes: { from: line.from, to: line.to, insert: line.text } });
  flushSync();
  expect(text(qa(".note-ed .cm-line")[0].querySelector(".cm-tm"))).toBe("10:00");
});

test("送信済みの印のある行を消したら、印を次の行へ移さずに外す", async () => {
  const { core, view, props } = await start("- 送った\n- 次");
  core.sends.push({ text: "- 送った", kind: "todo", itemId: "item-x" });
  props.items = "item-x";
  await until(() => sentLabels()[0] === "→ TODO");
  core.fail.marks = true;
  view().dispatch({ changes: { from: 0, to: view().state.doc.line(2).from } });
  flushSync();
  expect(sentLabels()).toEqual([""]);
  expect(sentCounts()).toEqual(["0", "0"]);
});

test("保存に失敗している間に行送りが本文を保存したら、「保存できていません」を終える", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  const { core, view } = await start("- a");
  // 保存が失敗する
  core.fail.save = true;
  view().dispatch({ changes: { from: 3, insert: "b" }, selection: { anchor: 4 }, userEvent: "input" });
  await vi.advanceTimersByTimeAsync(300);
  expect(tracker!.note.saver!.failure).not.toBeNull();
  // 行送りは成功し、本文を保存する
  q(".note-ed .cm-content")!.focus();
  press(T);
  await until(() => core.note.body === "- ab");
  await until(() => tracker!.note.saver!.failure === null && !tracker!.note.saver!.busy);
});

test("右クリックのメニューは、窓の右・下の端の近くでも窓の中に出す", async () => {
  await start("- a");
  const content = q(".note-ed .cm-content")!;
  content.focus();
  content.dispatchEvent(
    new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: window.innerWidth + 50, clientY: window.innerHeight + 50 }),
  );
  await until(() => q(".line-menu") !== null && document.activeElement === qa(".line-menu button")[0]);
  expect(parseFloat(q(".line-menu")!.style.left)).toBeLessThanOrEqual(window.innerWidth);
  expect(parseFloat(q(".line-menu")!.style.top)).toBeLessThanOrEqual(window.innerHeight);
});
