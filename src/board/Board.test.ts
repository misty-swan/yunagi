import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { SaveStatus } from "../lib/autosave.svelte";
import { EVENTS, type Board as BoardData, type BoardItem, type Item, type NoteSource } from "../lib/bindings";
import { Notice } from "../lib/notice.svelte";
import { prefs } from "../lib/prefs.svelte";
import { startSession, type Session } from "../lib/session.svelte";
import { InputTracker } from "../lib/ui_report.svelte";
import Board from "./Board.svelte";

const TODAY = "2026-11-03";
const A = "01JC00000000000000000000A0";
const B = "01JC00000000000000000000B0";
const C = "01JC00000000000000000000C0";
const D = "01JC00000000000000000000D0";

let app: ReturnType<typeof mount> | undefined;
let session: Session | undefined;
let tracker: InputTracker | undefined;

// jsdom には Range の位置が無い（CodeMirror が測るときに使う）
Range.prototype.getClientRects ??= () => [] as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();

beforeEach(() => {
  prefs.lang = "ja";
  prefs.layout = "A";
  prefs.vimMode = false;
});

afterEach(async () => {
  if (app) unmount(app);
  app = undefined;
  tracker?.stop();
  await session?.stop();
  session = undefined;
  document.body.innerHTML = "";
  await new Promise((r) => setTimeout(r, 0));
  clearMocks();
});

function item(id: string, text: string, over: Partial<Item> = {}): Item {
  return {
    id,
    date: TODAY,
    kind: "todo",
    text,
    status: "open",
    closeReason: null,
    sortKey: "a0",
    carryCount: 0,
    carriedFromId: null,
    splitFromId: null,
    resolutionNote: null,
    origin: "human",
    agentClientId: null,
    externalRefId: null,
    completedAt: null,
    createdAt: 0,
    updatedAt: 0,
    deletedAt: null,
    ...over,
  };
}

const bi = (i: Item, over: Partial<BoardItem> = {}): BoardItem => ({
  item: i,
  carriedTo: null,
  agentName: null,
  unseen: false,
  noteSource: null,
  ...over,
});

function emptyBoard(date: string): BoardData {
  return { date, todos: [], blockers: [], reflection: "", note: { headings: [], length: 0 }, reflectionRev: 0, noteRev: 0 };
}

interface Opts {
  boards?: Record<string, BoardData>;
  reflection?: string;
}

function mockCore(opts: Opts = {}) {
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  const boards = opts.boards ?? {};
  mockIPC(
    (cmd, args) => {
      const a = args as Record<string, unknown>;
      calls.push({ cmd, args: a });
      switch (cmd) {
        case "startup_status":
          return { state: "ready" };
        case "today":
          return TODAY;
        case "get_board":
          return boards[a.date as string] ?? emptyBoard(a.date as string);
        case "get_overview":
          return {
            counts: { todos: 0, done: 0, leftTodos: 0, leftBlockers: 0, resolved: 0, carried: 0 },
            recent: [],
            unseenChanges: 0,
          };
        case "get_text_doc":
          return { body: opts.reflection ?? "", rev: opts.reflection ? 1 : 0 };
        case "add_item":
          return item(D, a.text as string, { kind: a.kind as "todo" });
        case "save_item":
          return { updated: true, item: item(a.id as string, "x") };
        case "save_text_doc":
          return { result: "saved", rev: 2, updated: true };
        case "report_ui_state":
          return null;
      }
      return null;
    },
    { shouldMockEvents: true },
  );
  return calls;
}

async function until(check: () => boolean) {
  await vi.waitFor(() => {
    flushSync();
    expect(check()).toBe(true);
  });
}

const $ = (sel: string) => document.querySelector<HTMLElement>(sel);
const $$ = (sel: string) => [...document.querySelectorAll<HTMLElement>(sel)];
const text = (el: Element | null) => el?.textContent?.replace(/\s+/g, " ").trim() ?? "";

async function start(opts: Opts = {}, props: { narrow?: boolean; onOpenNoteSource?: (s: NoteSource) => void } = {}) {
  const calls = mockCore(opts);
  session = startSession();
  const notice = new Notice();
  const save = new SaveStatus();
  tracker = new InputTracker(save);
  app = mount(Board, {
    target: document.body,
    props: {
      session,
      save,
      notice,
      narrow: props.narrow ?? false,
      mac: false,
      tracker,
      onOpenNoteSource: props.onOpenNoteSource ?? (() => {}),
    },
  });
  await until(() => $(".board")?.dataset.date === TODAY && $(".ref-ed .cm-content") !== null);
  return { calls, notice };
}

test("3領域を、設定のレイアウトで並べ、900px 未満では縦積みにする", async () => {
  await start();
  expect($(".board")?.dataset.layout).toBe("A");
  expect($$(".board > .region, .board > section").map((r) => r.dataset.region)).toEqual(["todo", "blocker", "reflection"]);
  expect($$(".region h2").map(text)).toEqual(["TODO", "BLOCKERS", "REFLECTION"]);
  prefs.layout = "B";
  flushSync();
  expect($(".board")?.dataset.layout).toBe("B");
  unmount(app!);
  app = undefined;
  await session?.stop();
  await start({}, { narrow: true });
  expect($(".board")?.dataset.layout).toBe("C");
});

test("空の状態：TODO が空なら、今日は「今日やることを書く（Enter）」、ほかの日は「やることを書く（Enter）」", async () => {
  await start();
  expect(text($(".r-todo .empty"))).toBe("今日やることを書く（Enter）");
  expect($(".r-todo .list")).toBeNull();
  prefs.lang = "en";
  flushSync();
  expect(text($(".r-todo .empty"))).toBe("Write what to do today (Enter)");
  session!.show("2026-11-04");
  await until(() => $(".board")?.dataset.date === "2026-11-04");
  expect(text($(".r-todo .empty"))).toBe("Write what to do (Enter)");
});

test("持ち越し回数の印は、設定の見直しの回数以上で --warn にする", async () => {
  const board = {
    ...emptyBoard(TODAY),
    todos: [bi(item(A, "設計を読む", { carryCount: 2 }))],
    blockers: [bi(item("01JC00000000000000000000F0", "詰まり", { kind: "blocker", carryCount: 4 }))],
  };
  prefs.reviewThreshold = 3;
  try {
    await start({ boards: { [TODAY]: board } });
    expect($(".r-todo .carry")?.classList.contains("warn")).toBe(false);
    expect($(".r-blk .carry")?.classList.contains("warn")).toBe(true);
    prefs.reviewThreshold = 2;
    flushSync();
    expect($(".r-todo .carry")?.classList.contains("warn")).toBe(true);
    prefs.reviewThreshold = 5;
    flushSync();
    expect($(".r-blk .carry")?.classList.contains("warn")).toBe(false);
  } finally {
    prefs.reviewThreshold = 3;
  }
});

test("行の表示：件数、完了の取り消し線、解消メモ、持ち越し先、取りやめた理由、持ち越し回数、エージェント", async () => {
  await start({
    boards: {
      [TODAY]: {
        ...emptyBoard(TODAY),
        todos: [
          bi(item(A, "設計を読む", { carryCount: 3 })),
          bi(item(B, "レビュー", { status: "done", completedAt: 1 })),
          bi(item(C, "持ち越した"), { carriedTo: { id: D, date: "2026-11-04" } }),
          bi(item(D, "やめた", { status: "dropped", closeReason: "split" })),
          bi(item("01JC00000000000000000000E0", "足した", { origin: "agent" }), { agentName: "claude-code", unseen: true }),
        ],
        blockers: [
          bi(item("01JC00000000000000000000F0", "詰まり", { kind: "blocker", status: "resolved", resolutionNote: "聞いた" })),
          bi(item("01JC00000000000000000000G0", "まだ", { kind: "blocker" })),
        ],
      },
    },
  });
  expect(text($(".r-todo .rm"))).toBe("3 left");
  expect(text($(".r-blk .rm"))).toBe("1 open");
  const rows = $$(".r-todo .item");
  expect(rows.map((r) => text(r.querySelector(".txt")))).toEqual(["設計を読む", "レビュー", "持ち越した", "やめた", "足した"]);
  expect(rows[0].querySelector(".carry.warn")?.getAttribute("aria-label")).toBe("持ち越し 3 回");
  expect(rows[1].classList.contains("done")).toBe(true);
  expect(rows[1].querySelector(".chk")?.getAttribute("aria-pressed")).toBe("true");
  expect(text(rows[2].querySelector(".carried-to"))).toBe("→ 11.04");
  expect(rows[2].querySelector(".txt")?.getAttribute("contenteditable")).toBeNull();
  expect(rows[3].classList.contains("dropped")).toBe(true);
  expect(text(rows[3].querySelector(".cstate"))).toBe("分割した");
  expect(text(rows[4].querySelector(".agent-tag"))).toBe("Claude Code");
  expect(rows[4].querySelector(".mk-agent.lead")).not.toBeNull();
  expect(text($(".r-blk .item .memo"))).toBe("→ 聞いた");
});

test("ノートから送った項目は行末に「← NOTE」を出し、押すと送り元の行を知らせる", async () => {
  const opened: NoteSource[] = [];
  const source = { date: "2026-11-02", line: 3, itemId: B };
  await start(
    {
      boards: {
        [TODAY]: {
          ...emptyBoard(TODAY),
          todos: [bi(item(A, "送った", { origin: "note" }), { noteSource: source }), bi(item(C, "手で足した"))],
        },
      },
    },
    { onOpenNoteSource: (s) => opened.push(s) },
  );
  const rows = $$(".r-todo .item");
  expect(text(rows[0].querySelector(".from-note"))).toBe("← NOTE");
  expect(rows[0].querySelector(".from-note")?.getAttribute("aria-label")).toBe("ノートの送り元の行へ移る");
  expect(rows[1].querySelector(".from-note")).toBeNull();
  (rows[0].querySelector(".from-note") as HTMLElement).click();
  expect(opened).toEqual([source]);
});

test("持ち越し済みの項目を完了にしようとすると、上部のバーへの知らせを出し、変えない", async () => {
  const { calls, notice } = await start({
    boards: {
      [TODAY]: { ...emptyBoard(TODAY), todos: [bi(item(C, "持ち越した"), { carriedTo: { id: D, date: "2026-11-04" } })] },
    },
  });
  $(".r-todo .chk")!.click();
  flushSync();
  expect(notice.text).toBe("11.04 Wed へ持ち越し済みです");
  expect(calls.some((c) => c.cmd === "save_item")).toBe(false);
  expect($(".r-todo .item")?.classList.contains("done")).toBe(false);
});

test("持ち越し済みの項目と取りやめた TODO は、本文を変える操作（文字・Shift 付きの文字・削除キー・貼り付け・切り取り）を知らせて拒み、行の途中の Enter は分けずに直下に足す", async () => {
  const { calls, notice } = await start({
    boards: {
      [TODAY]: {
        ...emptyBoard(TODAY),
        todos: [
          bi(item(C, "持ち越した"), { carriedTo: { id: D, date: "2026-11-04" } }),
          bi(item(B, "やめた", { status: "dropped", closeReason: "dropped" })),
        ],
      },
    },
  });
  const tryKey = (row: HTMLElement, init: KeyboardEventInit) => {
    const e = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
    row.dispatchEvent(e);
    flushSync();
    return e.defaultPrevented;
  };
  const rows = $$(".r-todo .item");
  const keys: KeyboardEventInit[] = [
    { key: "a" },
    { key: "A", shiftKey: true },
    { key: " " },
    { key: "Backspace" },
    { key: "Delete" },
  ];
  const expected = ["11.04 Wed へ持ち越し済みです", "持ち越しの見直しで閉じた TODO は変更できません"];
  for (const [i, row] of rows.entries()) {
    for (const init of keys) {
      notice.text = "";
      expect(tryKey(row, init)).toBe(true);
      expect(notice.text).toBe(expected[i]);
    }
    // jsdom には DragEvent が無いので、運ぶものの種類（dataTransfer.types）を足した Event で確かめる
    const drag = (type: string, types: string[]) => {
      const e = new Event(type, { bubbles: true, cancelable: true });
      Object.defineProperty(e, "dataTransfer", { value: { types } });
      row.dispatchEvent(e);
      flushSync();
      return e;
    };
    for (const type of ["paste", "cut"]) {
      notice.text = "";
      const e = new Event(type, { bubbles: true, cancelable: true });
      row.dispatchEvent(e);
      flushSync();
      expect(e.defaultPrevented).toBe(true);
      expect(notice.text).toBe(expected[i]);
    }
    for (const types of [["text/plain"], ["Files"]]) {
      notice.text = "";
      expect(drag("dragover", types).defaultPrevented).toBe(true);
      expect(drag("drop", types).defaultPrevented).toBe(true);
      expect(notice.text).toBe(expected[i]);
    }
    // 運ぶものが無い drag（行の並べ替えなど）は、受けず知らせない
    notice.text = "";
    expect(drag("dragover", []).defaultPrevented).toBe(false);
    expect(drag("drop", []).defaultPrevented).toBe(false);
    expect(notice.text).toBe("");
    // 貼り付け・切り取りのキー、日本語入力の変換、AltGr（Ctrl+Alt）で打つ文字も知らせる
    for (const init of [
      { key: "v", code: "KeyV", ctrlKey: true },
      { key: "x", code: "KeyX", ctrlKey: true },
      { key: "Process", keyCode: 229 },
      { key: "@", ctrlKey: true, altKey: true },
    ]) {
      notice.text = "";
      tryKey(row, init);
      expect(notice.text).toBe(expected[i]);
    }
    // Option+Backspace も拒む。Mod との組み合わせ（コピーなど）と、Windows・Linux の Alt+文字は知らせず、止めない
    notice.text = "";
    expect(tryKey(row, { key: "Backspace", altKey: true })).toBe(true);
    expect(notice.text).toBe(expected[i]);
    for (const init of [{ key: "c", code: "KeyC", ctrlKey: true }, { key: "f", altKey: true }]) {
      notice.text = "";
      expect(tryKey(row, init)).toBe(false);
      expect(notice.text).toBe("");
    }
    // 行の中のボタンのキーは、行が奪わない
    const btn = row.querySelector<HTMLElement>(".chk")!;
    const inner = new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true });
    btn.dispatchEvent(inner);
    expect(inner.defaultPrevented).toBe(false);
  }
  expect(calls.some((c) => c.cmd === "save_item" || c.cmd === "add_item")).toBe(false);
  expect(rows.map((r) => text(r.querySelector(".txt")))).toEqual(["持ち越した", "やめた"]);
  // 行の途中の Enter は、本文を変えないので知らせず、分けずに直下へ空の項目を足す
  notice.text = "";
  expect(tryKey(rows[0], { key: "Enter" })).toBe(true);
  expect(notice.text).toBe("");
  await until(() => $$(".r-todo .item").length === 3);
  expect($$(".r-todo .item").map((r) => text(r.querySelector(".txt")))).toEqual(["持ち越した", "", "やめた"]);
});

/** Vim のノーマルモードのキー（行そのものへ）。 */
function vimKey(row: HTMLElement | null, key: string, init: KeyboardEventInit = {}) {
  const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  row?.dispatchEvent(e);
  flushSync();
  return e;
}

const rowsOf = (sel: string) => $$(`${sel} .item`).map((r) => text(r.querySelector(".txt")));

test("Vim モードの整理モードのリスト：ノーマルモードは行を選ぶだけで、j / k / gg / G で移り、o で足して挿入モード、Esc で戻る", async () => {
  prefs.vimMode = true;
  const { calls } = await start({
    boards: {
      [TODAY]: {
        ...emptyBoard(TODAY),
        todos: [bi(item(A, "一", { sortKey: "a0" })), bi(item(B, "二", { sortKey: "a1" })), bi(item(C, "三", { sortKey: "a2" }))],
      },
    },
  });
  const rows = () => $$(".r-todo .item");
  // ノーマルモードの本文は入力欄でない
  expect($$(".r-todo [contenteditable]").length).toBe(0);
  rows()[0].focus();
  const active = () => document.activeElement as HTMLElement;
  vimKey(active(), "j");
  await until(() => active() === rows()[1]);
  vimKey(active(), "j");
  vimKey(active(), "j");
  await until(() => active() === rows()[2]);
  vimKey(active(), "k");
  await until(() => active() === rows()[1]);
  vimKey(active(), "g");
  vimKey(active(), "g");
  await until(() => active() === rows()[0]);
  vimKey(active(), "G");
  await until(() => active() === rows()[2]);
  // g の後に別のキーなら、g は捨てる（次の g では移らない）
  vimKey(active(), "g");
  vimKey(active(), "k");
  await until(() => active() === rows()[1]);
  vimKey(active(), "g");
  expect(active()).toBe(rows()[1]);
  // 文字のキーは入力にならない
  expect(vimKey(active(), "z").defaultPrevented).toBe(true);
  expect(rowsOf(".r-todo")).toEqual(["一", "二", "三"]);

  // o：直下に足して挿入モード（入力欄になる）。Esc で行そのものに戻り、ノーマルモードになる
  vimKey(active(), "o");
  await until(() => $$(".r-todo .item").length === 4 && $(".r-todo [contenteditable]") !== null);
  const added = $$(".r-todo .item")[2];
  await until(() => added.contains(document.activeElement));
  expect(document.activeElement?.getAttribute("contenteditable")).toBe("true");
  document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  flushSync();
  await until(() => $$(".r-todo [contenteditable]").length === 0);
  expect(active()).toBe(added);
  // 何も書かずに離れた行は消える
  vimKey(active(), "j");
  await until(() => $$(".r-todo .item").length === 3);
  expect(calls.some((c) => c.cmd === "add_item")).toBe(false);
});

test("Vim モードの整理モードのリスト：i / a で編集（i は行頭、a は行末）、x で完了の切替、dd で削除、J / K で並べ替え、Ctrl+[ でも戻る", async () => {
  prefs.vimMode = true;
  const { calls } = await start({
    boards: {
      [TODAY]: {
        ...emptyBoard(TODAY),
        todos: [bi(item(A, "一", { sortKey: "a0" })), bi(item(B, "二", { sortKey: "a1" }))],
      },
    },
  });
  const rows = () => $$(".r-todo .item");
  rows()[0].focus();
  const active = () => document.activeElement as HTMLElement;
  // x：完了の切替
  vimKey(active(), "x");
  await until(() => rows()[0].classList.contains("done"));
  vimKey(active(), "x");
  await until(() => !rows()[0].classList.contains("done"));
  // i：編集を始める（入力欄になり、カーソルは行頭）
  vimKey(active(), "i");
  await until(() => document.activeElement?.getAttribute("contenteditable") === "true");
  expect(window.getSelection()?.anchorOffset).toBe(0);
  // Windows・Linux の Ctrl+[ は Esc と同じ
  document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "[", code: "BracketLeft", ctrlKey: true, bubbles: true, cancelable: true }));
  flushSync();
  await until(() => $$(".r-todo [contenteditable]").length === 0 && active() === rows()[0]);
  // a：行末
  vimKey(active(), "a");
  await until(() => document.activeElement?.getAttribute("contenteditable") === "true");
  expect(window.getSelection()?.anchorOffset).toBe(1);
  document.activeElement!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  flushSync();
  await until(() => active() === rows()[0]);
  // J：下へ動かす。K：上へ動かす
  vimKey(active(), "J");
  await until(() => rowsOf(".r-todo").join() === "二,一");
  vimKey(active(), "K");
  await until(() => rowsOf(".r-todo").join() === "一,二");
  // dd：削除（d だけでは消えない）。次の行を選ぶ
  vimKey(active(), "d");
  expect(rows().length).toBe(2);
  vimKey(active(), "d");
  await until(() => rows().length === 1);
  expect(rowsOf(".r-todo")).toEqual(["二"]);
  await until(() => calls.some((c) => c.cmd === "delete_item"));
  // u：取り消す
  vimKey(active(), "u");
  await until(() => rows().length === 2);
});

test("Vim モードで、持ち越し済みの項目と取りやめた TODO では、i / a / x が本文・状態を変えず、上部のバーに知らせる。o / O は足すだけで、dd と J / K は受ける", async () => {
  prefs.vimMode = true;
  const { calls, notice } = await start({
    boards: {
      [TODAY]: {
        ...emptyBoard(TODAY),
        todos: [
          bi(item(C, "持ち越した", { sortKey: "a0" }), { carriedTo: { id: D, date: "2026-11-04" } }),
          bi(item(B, "やめた", { sortKey: "a1", status: "dropped", closeReason: "dropped" })),
        ],
      },
    },
  });
  const rows = () => $$(".r-todo .item");
  const expected = ["11.04 Wed へ持ち越し済みです", "持ち越しの見直しで閉じた TODO は変更できません"];
  for (const [i, row] of rows().entries()) {
    for (const key of ["i", "a", "x"]) {
      notice.text = "";
      row.focus();
      expect(vimKey(row, key).defaultPrevented).toBe(true);
      expect(notice.text, key).toBe(expected[i]);
      // 挿入モードに入らない（入力欄にならない）
      expect($$(".r-todo [contenteditable]").length).toBe(0);
    }
    // 文字でない j k や、操作に当たらない文字では知らせない
    for (const key of ["j", "k", "z"]) {
      notice.text = "";
      vimKey(row, key);
      expect(notice.text, key).toBe("");
    }
  }
  expect(calls.some((c) => c.cmd === "save_item" || c.cmd === "add_item")).toBe(false);
  expect(rowsOf(".r-todo")).toEqual(["持ち越した", "やめた"]);
  expect(rows().map((r) => r.classList.contains("done"))).toEqual([false, true]);

  // o：直下に空の項目を足して挿入モード（知らせない）。足した項目は通常の項目
  notice.text = "";
  rows()[0].focus();
  vimKey(rows()[0], "o");
  await until(() => rows().length === 3 && $(".r-todo [contenteditable]") !== null);
  expect(notice.text).toBe("");
  expect(rowsOf(".r-todo")).toEqual(["持ち越した", "", "やめた"]);
});

test("Vim モードでない間は、整理モードのリストの本文は常に入力欄", async () => {
  await start({ boards: { [TODAY]: { ...emptyBoard(TODAY), todos: [bi(item(A, "一"))] } } });
  expect($$(".r-todo [contenteditable]").length).toBe(1);
});

test("持ち越し済みの障害の解消メモを押しても、入力欄を開かず知らせる", async () => {
  const { notice } = await start({
    boards: {
      [TODAY]: {
        ...emptyBoard(TODAY),
        blockers: [
          bi(item(A, "詰まり", { kind: "blocker", status: "resolved", resolutionNote: "聞いた" }), {
            carriedTo: { id: D, date: "2026-11-04" },
          }),
        ],
      },
    },
  });
  $(".r-blk .item .memo")!.click();
  flushSync();
  expect($(".r-blk .memo-edit")).toBeNull();
  expect(notice.text).toBe("11.04 Wed へ持ち越し済みです");
});

test("空の TODO で Enter を押して入力すると項目を作り、チェック欄で完了にする", async () => {
  const { calls } = await start();
  $(".r-todo .empty")!.click();
  await until(() => $(".r-todo .item .txt") !== null);
  const txt = $(".r-todo .item .txt")!;
  expect(document.activeElement).toBe(txt);
  txt.textContent = "設計を読む";
  txt.dispatchEvent(new Event("input", { bubbles: true }));
  await until(() => calls.some((c) => c.cmd === "add_item"));
  expect(calls.find((c) => c.cmd === "add_item")?.args).toEqual({
    date: TODAY,
    kind: "todo",
    text: "設計を読む",
    position: { at: "first" },
  });
  // 入力中の項目の欄を知らせる
  await until(() =>
    calls.some(
      (c) =>
        c.cmd === "report_ui_state" &&
        JSON.stringify((c.args.state as { editingItems: unknown[] }).editingItems).includes(D),
    ),
  );
  $(".r-todo .chk")!.click();
  await until(() => calls.some((c) => c.cmd === "save_item"));
  expect($(".r-todo .item")?.classList.contains("done")).toBe(true);
});

test("エージェントの変更（changed）で読み込み直し、入力中の行の本文は画面の値を残す", async () => {
  const boards: Record<string, BoardData> = {
    [TODAY]: { ...emptyBoard(TODAY), todos: [bi(item(A, "設計を読む")), bi(item(B, "レビュー"))] },
  };
  await start({ boards });
  const txt = $$(".r-todo .item .txt")[0];
  txt.focus();
  txt.textContent = "設計書を読む";
  txt.dispatchEvent(new Event("input", { bubbles: true }));
  boards[TODAY] = { ...emptyBoard(TODAY), todos: [bi(item(A, "設計を読む")), bi(item(B, "レビューする"))] };
  await emit(EVENTS.changed, { by: "agent" });
  await until(() => text($$(".r-todo .item .txt")[1]) === "レビューする");
  expect(text($$(".r-todo .item .txt")[0])).toBe("設計書を読む");
});

test("振り返りの「## 」の行は見出しとして出し、Mod+Shift+R でテンプレートを入れる", async () => {
  const { calls } = await start({ reflection: "## 良かったこと\n- 早く終えた" });
  const lines = $$(".ref-ed .cm-line");
  expect(lines[0].classList.contains("cm-ref-h")).toBe(true);
  expect(text(lines[0].querySelector(".cm-md"))).toBe("##");
  expect(lines[1].classList.contains("cm-ref-h")).toBe(false);
  const content = $(".ref-ed .cm-content")!;
  content.focus();
  content.dispatchEvent(
    new KeyboardEvent("keydown", { key: "R", code: "KeyR", keyCode: 82, ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true }),
  );
  await until(() => calls.some((c) => c.cmd === "save_text_doc"));
  // カーソルのある行（先頭の行）の次の行から入れる。もとの行は残る
  const body = calls.find((c) => c.cmd === "save_text_doc")?.args.body as string;
  expect(body.split("\n")).toEqual([
    "## 良かったこと",
    "## 良かったこと",
    "- ",
    "",
    "## 詰まったこと",
    "- ",
    "",
    "## 明日の一手",
    "- ",
    "- 早く終えた",
  ]);
});

test("振り返りを入力してすぐ別の日へ移っても、前の日には入力した本文を保存する", async () => {
  const { calls } = await start({ reflection: "はじめ" });
  const content = $(".ref-ed .cm-content")!;
  // CodeMirror に入力を渡す（文字を足して input を起こす）
  const line = content.querySelector(".cm-line")!;
  line.textContent = "はじめ、つづき";
  content.dispatchEvent(new Event("input", { bubbles: true }));
  await until(() => true);
  session!.show("2026-11-04");
  await until(() => calls.some((c) => c.cmd === "save_text_doc" && c.args.date === TODAY));
  const saves = calls.filter((c) => c.cmd === "save_text_doc" && c.args.date === TODAY).map((c) => c.args.body);
  expect(saves).not.toContain("");
});
