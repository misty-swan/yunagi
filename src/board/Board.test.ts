import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { SaveStatus } from "../lib/autosave.svelte";
import { EVENTS, type Board as BoardData, type BoardItem, type Item } from "../lib/bindings";
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
  ...over,
});

function emptyBoard(date: string): BoardData {
  return { date, todos: [], blockers: [], reflection: "", note: { headings: [], length: 0 } };
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

async function start(opts: Opts = {}, props: { narrow?: boolean } = {}) {
  const calls = mockCore(opts);
  session = startSession();
  const notice = new Notice();
  const save = new SaveStatus();
  tracker = new InputTracker(save);
  app = mount(Board, {
    target: document.body,
    props: { session, save, notice, narrow: props.narrow ?? false, mac: false, tracker },
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
