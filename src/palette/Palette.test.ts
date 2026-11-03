// 検索パレット（S-04。画面設計書 6章）を、アプリ全体の画面で確かめる（コアは模擬）。

import { EditorView } from "@codemirror/view";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import App from "../App.svelte";
import type { Board, BoardItem, CarryoverStatus, Counts, Item, Overview, SearchHit } from "../lib/bindings";
import { SEARCH_DELAY_MS } from "../lib/palette.svelte";
import { prefs } from "../lib/prefs.svelte";

let app: ReturnType<typeof mount> | undefined;

Range.prototype.getClientRects ??= () => [] as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();

const TODAY = "2026-11-03";
const OLD = "2025-10-28";

beforeEach(() => {
  prefs.lang = "ja";
  prefs.layout = "A";
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1280 });
});

afterEach(async () => {
  if (app) unmount(app);
  app = undefined;
  document.body.innerHTML = "";
  await new Promise((resolve) => setTimeout(resolve, 0));
  clearMocks();
});

function counts(): Counts {
  return { todos: 0, done: 0, leftTodos: 0, leftBlockers: 0, resolved: 0, carried: 0 };
}

function item(id: string, date: string, text: string, over: Partial<Item> = {}): Item {
  return {
    id,
    date,
    kind: "todo",
    text,
    status: "open",
    closeReason: null,
    sortKey: id,
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

function boardItem(i: Item): BoardItem {
  return { item: i, carriedTo: null, agentName: null, unseen: false, noteSource: null };
}

const NOTE = "朝の確認\n## 10:00 定例\n- レート制限の件を共有";
const REFLECTION = "## 良かったこと\n- レート制限を早めに見つけた";

function board(date: string): Board {
  const todos =
    date === OLD
      ? [boardItem(item("01JA", OLD, "ほかの TODO")), boardItem(item("01JB", OLD, "API のレート制限を調べる"))]
      : [boardItem(item("01JT", TODAY, "今日の TODO"))];
  return { date, todos, blockers: [], reflection: "", note: { headings: [], length: 0 }, reflectionRev: 1, noteRev: 1 };
}

const HITS: SearchHit[] = [
  { kind: "note", refId: TODAY, date: TODAY, body: NOTE, resolutionNote: null, carriedTo: null },
  { kind: "reflection", refId: OLD, date: OLD, body: REFLECTION, resolutionNote: null, carriedTo: null },
  {
    kind: "todo",
    refId: "01JB",
    date: OLD,
    body: "API のレート制限を調べる",
    resolutionNote: null,
    carriedTo: { id: "01JC", date: "2025-10-29" },
  },
];

interface Fake {
  status: CarryoverStatus | null;
  /** 検索の答えを返すまでの時間（ミリ秒）。 */
  searchDelay?: number;
}

function mockCore(fake: Fake = { status: null }) {
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  mockIPC(
    async (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args: a });
      switch (cmd) {
        case "startup_status":
          return { state: "ready" };
        case "today":
          return TODAY;
        case "get_board":
          return board(a.date as string);
        case "get_text_doc":
          return { body: a.doc === "note" && a.date === TODAY ? NOTE : a.doc === "reflection" && a.date === OLD ? REFLECTION : "", rev: 1 };
        case "get_overview": {
          const o: Overview = { counts: counts(), recent: [{ date: TODAY, counts: counts() }], unseenChanges: 0 };
          return o;
        }
        case "carryover_status":
          return fake.status;
        case "carryover_candidates":
          return { from: "2026-11-02", items: [{ item: item("01JP", "2026-11-02", "残った TODO"), review: false }], reviewThreshold: 3 };
        case "postpone_carryover":
        case "report_ui_state":
          return null;
        case "note_marks":
          return [];
        case "search": {
          if (fake.searchDelay) await new Promise((r) => setTimeout(r, fake.searchDelay));
          const text = a.text as string;
          return HITS.filter((h) => h.body.includes(text));
        }
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
const input = () => $("[data-palette] input")! as HTMLInputElement;
/** グループの見出しと行（表示の順）。 */
const rows = () => $$("#pal-list > li").map((li) => (li.classList.contains("group") ? `# ${text(li)}` : text(li)));
const selected = () => text($("#pal-list [aria-selected='true']"));
/** 背後を操作できなくしているか（jsdom は inert の属性を付けないので、プロパティを見る）。 */
function inert(el: Element | null): boolean {
  for (let e = el; e !== null; e = e.parentElement) if ((e as HTMLElement).inert) return true;
  return false;
}

function press(init: KeyboardEventInit, target: EventTarget = document.activeElement ?? window) {
  target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  flushSync();
}

function type(value: string) {
  const el = input();
  el.value = value;
  el.dispatchEvent(new InputEvent("input", { bubbles: true }));
  flushSync();
}

async function start(fake?: Fake) {
  const calls = mockCore(fake);
  app = mount(App, { target: document.body });
  await until(() => $("main.content")?.dataset.date === TODAY && $(".meter") !== null);
  return calls;
}

async function openWithKey() {
  press({ key: "k", code: "KeyK", ctrlKey: true }, window);
  await until(() => document.activeElement === $("[data-palette] input"));
}

test("Mod+K と上部のバーの検索欄で開き、空のときはコマンドの一覧を出す。Esc・Mod+K で閉じて元の所へ戻る", async () => {
  await start();
  // jsdom の click() はフォーカスを移さないので、押す前に移す（実際の WebView ではボタンに移る）
  $(".search")!.focus();
  $(".search")!.click();
  await until(() => document.activeElement === $("[data-palette] input"));
  expect($("[data-palette]")?.getAttribute("role")).toBe("dialog");
  // 背後は操作できない
  expect(inert($("main.content"))).toBe(true);
  expect(rows()).toEqual([
    "# COMMANDS",
    "今日へ移動 Ctrl+T",
    "ノートモードにする Ctrl+J",
    "ふりかえり（Archive）を開く Ctrl+Shift+L",
    "エージェントの変更履歴を開く",
    "設定を開く Ctrl+,",
    "サイドバーを閉じる Ctrl+\\",
    "レイアウト：A. 2段 使用中",
    "レイアウト：B. 3列",
    "レイアウト：C. 縦積み",
  ]);
  expect(selected()).toBe("今日へ移動 Ctrl+T");
  press({ key: "Escape" });
  await until(() => $("[data-palette]") === null);
  expect(document.activeElement).toBe($(".search"));
  expect(inert($("main.content"))).toBe(false);

  await openWithKey();
  press({ key: "k", code: "KeyK", ctrlKey: true });
  await until(() => $("[data-palette]") === null);
});

test("入力すると、日付へ移動・コマンド・検索結果の順に、種類・日付・持ち越し先と一致した所を出す", async () => {
  await start();
  await openWithKey();
  type("レート");
  await until(() => rows().length === 4);
  expect(rows()).toEqual([
    "# RESULTS",
    "2026.11.03 Tue NOTE - レート制限の件を共有",
    "2025.10.28 Tue REFLECTION - レート制限を早めに見つけた",
    "2025.10.28 Tue TODO API のレート制限を調べる → 10.29 Wed",
  ]);
  expect($$("#pal-list mark").map((m) => m.textContent)).toEqual(["レート", "レート", "レート"]);

  // 日付は日付へ移動に。コマンドは名前で絞る
  type("昨日");
  await until(() => rows()[0] === "# GO TO DATE");
  expect(rows().slice(0, 2)).toEqual(["# GO TO DATE", "2026.11.02 Mon を開く"]);
  type("レイアウト");
  await until(() => rows()[0] === "# COMMANDS");
  expect(rows().slice(0, 4)).toEqual(["# COMMANDS", "レイアウト：A. 2段 使用中", "レイアウト：B. 3列", "レイアウト：C. 縦積み"]);
  type("見つからない語");
  await until(() => text($("[data-palette] .state")) === "一致するものはありません。");
});

test("日付ジャンプ：その日へ移り、モードは変えない", async () => {
  await start();
  await openWithKey();
  type("2025-10-28");
  await until(() => selected() === "2025.10.28 Tue を開く");
  press({ key: "Enter" });
  await until(() => $("main.content")?.dataset.date === OLD);
  expect($("[data-palette]")).toBeNull();
  expect($("main.content")?.dataset.mode).toBe("board");
});

test("項目の結果を選ぶと、その日のボードのその項目へ移る", async () => {
  await start();
  // ノートモードから選んでも、整理モードへ移る
  press({ key: "j", code: "KeyJ", ctrlKey: true }, window);
  await until(() => $("main.content")?.dataset.mode === "note");
  await openWithKey();
  type("API");
  await until(() => rows().some((r) => r.startsWith("2025.10.28 Tue TODO")));
  press({ key: "ArrowDown" });
  press({ key: "ArrowUp" });
  // ↓ ↑ で移る（コマンドの後の結果まで）
  while (!selected().startsWith("2025.10.28 Tue TODO")) press({ key: "ArrowDown" });
  press({ key: "Enter" });
  await until(() => $("main.content")?.dataset.date === OLD && $("main.content")?.dataset.mode === "board");
  await until(() => document.activeElement?.closest("li[data-key]")?.textContent?.includes("API のレート制限を調べる") === true);
});

test("ノート・振り返りの結果を選ぶと、その日のノート・振り返りの一致した所を選ぶ", async () => {
  await start();
  await openWithKey();
  type("レート");
  await until(() => selected().startsWith("2026.11.03 Tue NOTE"));
  press({ key: "Enter" });
  await until(() => $("main.content")?.dataset.mode === "note");
  await until(() => editorSelection().region === "note" && editorSelection().text === "レート");

  await openWithKey();
  type("レート");
  await until(() => rows().some((r) => r.startsWith("2025.10.28 Tue REFLECTION")));
  press({ key: "ArrowDown" });
  press({ key: "Enter" });
  await until(() => $("main.content")?.dataset.date === OLD && $("main.content")?.dataset.mode === "board");
  await until(() => editorSelection().region === "reflection" && editorSelection().text === "レート");
});

/** フォーカスのあるエディタと、その選択範囲の文字。 */
function editorSelection(): { region: string | null; text: string } {
  const host = document.activeElement?.closest<HTMLElement>(".cm-editor") ?? null;
  const view = host === null ? null : EditorView.findFromDOM(host);
  if (view === null) return { region: null, text: "" };
  const { from, to } = view.state.selection.main;
  const region = host!.closest("[data-region='reflection']") !== null ? "reflection" : "note";
  return { region, text: view.state.sliceDoc(from, to) };
}

test("「持ち越しを開く」は今日の持ち越しが残っている間だけ出し、選ぶと持ち越しの画面を開く", async () => {
  const fake: Fake = { status: { from: "2026-11-02", prompt: "on_request" } };
  await start(fake);
  await openWithKey();
  await until(() => rows()[1] === "持ち越しを開く");
  press({ key: "Enter" });
  await until(() => $("[data-carryover]")?.dataset.carryover === "2026-11-02");
  expect($("[data-palette]")).toBeNull();
  // 後回しで閉じると、パレットを開く前の所へ戻る
  press({ key: "Escape" });
  await until(() => $("[data-carryover]") === null);

  fake.status = null;
  window.dispatchEvent(new Event("focus"));
  await openWithKey();
  await until(() => rows()[1] === "今日へ移動 Ctrl+T");
});

test("パレットを開いている間は、ノートの Mod+Shift+H と骨組みのキーを受けない（行にフォーカスがあるときも）", async () => {
  const calls = await start();
  press({ key: "j", code: "KeyJ", ctrlKey: true }, window);
  await until(() => $("main.content")?.dataset.mode === "note");
  await openWithKey();
  const before = calls.filter((c) => c.cmd === "save_text_doc").length;
  const H = { key: "H", code: "KeyH", keyCode: 72, ctrlKey: true, shiftKey: true };
  press(H);
  // 行の上（クリックなどで行にフォーカスが移ったとき）
  const row = $("#pal-list li[role='option']")!;
  press(H, row);
  press({ key: "j", code: "KeyJ", ctrlKey: true }, row);
  // 背後にフォーカスが落ちたとき
  press(H, document.body);
  await new Promise((r) => setTimeout(r, 400));
  flushSync();
  expect(calls.filter((c) => c.cmd === "save_text_doc").length).toBe(before);
  expect($("main.content")?.dataset.mode).toBe("note");
  expect($("[data-palette]")).not.toBeNull();
  expect(document.activeElement).toBe(input());
});

test("1文字ずつ打っても、入力欄はすぐ変わり、検索は打ち止めてから1回だけ送る", async () => {
  const calls = await start({ status: null, searchDelay: 30 });
  await openWithKey();
  const word = "API のレート制限を調べる";
  for (let i = 1; i <= word.length; i++) {
    type(word.slice(0, i));
    expect(input().value).toBe(word.slice(0, i));
    await new Promise((r) => setTimeout(r, SEARCH_DELAY_MS / 4));
  }
  await until(() => rows().some((r) => r.startsWith("2025.10.28 Tue TODO")));
  await new Promise((r) => setTimeout(r, SEARCH_DELAY_MS * 2));
  const searches = calls.filter((c) => c.cmd === "search").map((c) => c.args.text);
  expect(searches).toEqual([word]);
});
