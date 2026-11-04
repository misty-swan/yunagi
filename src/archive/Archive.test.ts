// ふりかえり（S-08。画面設計書 9章）：アプリ全体を描き、開閉・週と日の移動・Enter で開く・Esc で戻る・
// パレットとサイドバーからの動き・持ち越しとの関係を確かめる。コアは模擬。
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import App from "../App.svelte";
import { EVENTS, type Archive, type Counts } from "../lib/bindings";
import { addDays, weekday } from "../lib/dates";
import { prefs } from "../lib/prefs.svelte";

let app: ReturnType<typeof mount> | undefined;

// jsdom には Range の位置が無い（CodeMirror が測るときに使う）
Range.prototype.getClientRects ??= () => [] as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();

const TODAY = "2026-11-04"; // 水曜
const state = { today: TODAY, carry: null as unknown, failArchive: false };

beforeEach(() => {
  prefs.lang = "ja";
  state.today = TODAY;
  state.carry = null;
  state.failArchive = false;
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1280 });
  window.dispatchEvent(new Event("resize"));
});

afterEach(async () => {
  if (app) unmount(app);
  app = undefined;
  document.body.innerHTML = "";
  await new Promise((resolve) => setTimeout(resolve, 0));
  clearMocks();
});

function counts(c: Partial<Counts>): Counts {
  return { todos: 0, done: 0, leftTodos: 0, leftBlockers: 0, resolved: 0, carried: 0, ...c };
}

/** 日付 `date` を含む週の月曜から金曜。 */
function weekOf(date: string): string[] {
  const back = (new Date(`${date}T00:00:00Z`).getUTCDay() + 6) % 7;
  const monday = addDays(date, -back);
  return [0, 1, 2, 3, 4].map((i) => addDays(monday, i));
}

const DAY_COUNTS: Record<string, Counts> = {
  "2026-11-02": counts({ todos: 4, done: 3, leftTodos: 1, resolved: 1 }),
  "2026-11-03": counts({ todos: 2, done: 2, carried: 1 }),
  "2026-11-04": counts({ todos: 5, done: 1, leftTodos: 4, leftBlockers: 1, carried: 2 }),
};
const REFLECTIONS: Record<string, string> = {
  "2026-11-03": "## 良かったこと\n- 設計を終えた\n## 詰まったこと\n- CI が落ちる",
  "2026-10-30": "## Went well\n- shipped\n## Got stuck\n- flaky tests",
};

function archiveOf(date: string | null): Archive {
  const sel = date === null ? "2026-11-03" : date > state.today ? state.today : date;
  const week = weekOf(sel).map((d) => ({ date: d, counts: DAY_COUNTS[d] ?? counts({}) }));
  const sum = (k: keyof Counts) => week.reduce((n, d) => n + d.counts[k], 0);
  return {
    date: sel,
    today: state.today,
    week,
    weekTotal: counts({ done: sum("done"), carried: sum("carried"), resolved: sum("resolved") }),
    daily: [state.today, "2026-11-03", "2026-10-30"].map((d) => ({
      date: d,
      counts: DAY_COUNTS[d] ?? counts({ todos: 1, done: 1 }),
      reflection: REFLECTIONS[d] ?? "",
    })),
    preview: {
      counts: DAY_COUNTS[sel] ?? counts({}),
      reflection: REFLECTIONS[sel] ?? "",
      note:
        sel === "2026-11-03"
          ? [{ hour: 9, minute: 30, title: "定例", lines: ["- 議事1", "議事2"] }]
          : [],
    },
  };
}

function mockCore() {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args: a });
      switch (cmd) {
        case "startup_status":
          return { state: "ready" };
        case "today":
          return state.today;
        case "get_board":
          return { date: a.date, todos: [], blockers: [], reflection: "", note: { headings: [], length: 0 }, reflectionRev: 0, noteRev: 0 };
        case "get_text_doc":
          return { body: "", rev: 0 };
        case "report_ui_state":
          return null;
        case "carryover_status":
          return state.carry;
        case "carryover_candidates":
          return { from: "2026-11-03", reviewThreshold: 3, items: [] };
        case "get_overview":
          return {
            counts: DAY_COUNTS[TODAY],
            recent: [
              { date: state.today, counts: DAY_COUNTS[TODAY] },
              { date: "2026-11-03", counts: DAY_COUNTS["2026-11-03"] },
            ],
            unseenChanges: 0,
          };
        case "get_archive":
          if (state.failArchive) throw { kind: "storage", message: "x" };
          return archiveOf(a.date as string | null);
      }
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
const text = (sel: string) => $(sel)?.textContent?.replace(/\s+/g, " ").trim() ?? "";
const hints = () =>
  $$(".hints li")
    .map((li) => li.textContent?.replace(/\s+/g, " ").trim())
    .join(" | ");

function press(init: KeyboardEventInit, target: EventTarget = window) {
  target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  flushSync();
}

const MOD_SHIFT_L = { key: "L", code: "KeyL", ctrlKey: true, shiftKey: true };
const MOD_LEFT = { key: "[", code: "BracketLeft", ctrlKey: true };
const MOD_RIGHT = { key: "]", code: "BracketRight", ctrlKey: true };

async function start() {
  const calls = mockCore();
  app = mount(App, { target: document.body });
  await until(() => $("main.content")?.dataset.date === TODAY && $(".meter") !== null);
  return calls;
}

const archiveCalls = (calls: { cmd: string; args: unknown }[]) =>
  calls.filter((c) => c.cmd === "get_archive").map((c) => (c.args as { date: string | null }).date);
const selected = () => $("[role='option'][aria-selected='true'] .cap")?.textContent?.replace(/\s+/g, " ").trim();

/** プレビューの振り返りとノートの行（` | ` でつなぐ）。 */
const previewLines = () =>
  $$(".pcols .label, .pcols .ph2, .pcols .pl, .pcols .nh")
    .map((e) => e.textContent?.replace(/\s+/g, " ").trim())
    .join(" | ");
const paletteInput =() => $("[data-palette] input") as HTMLInputElement;
const paletteSelected = () => text("#pal-list [aria-selected='true']");
async function openPalette() {
  press({ key: "k", code: "KeyK", ctrlKey: true });
  await until(() => document.activeElement === $("[data-palette] input"));
}

async function openArchive() {
  press(MOD_SHIFT_L);
  await until(() => $(".archive .card") !== null);
}

test("Mod+Shift+L で開き、上部のバーは Archive の見出しになり、凪メーターは出さない。前の日を選んで始める", async () => {
  const calls = await start();
  await openArchive();
  // 今日を見ているので、コアが決める（今日より前で記録のある最も新しい日）
  expect(archiveCalls(calls)).toEqual([null]);
  expect(text(".top .archive-title")).toBe("Archive");
  expect($(".top .datenav")).toBeNull();
  expect($(".top .seg")).toBeNull();
  expect($(".meter")).toBeNull();
  expect(hints()).toBe("← → Day | Ctrl+[ Ctrl+] Week | ↵ Open | Esc Back");
  expect($(".side .nav-btn[aria-current='page']")?.textContent).toContain("Archive");
  // 整理モードとノートは消さずに隠す
  expect($$(".content > .mode").filter((m) => m.hidden).length).toBe(2);

  // This week：選んだ日（11-03）を含む週の範囲と合計
  expect(text(".archive .head")).toBe("This week 11.02 Mon — 11.06 Fri 6 Done · 3 Carried · 1 Resolved");
  const cards = $$(".archive .card");
  expect(cards.length).toBe(5);
  expect(selected()).toBe("11.03 Tue 2/2");
  // 今日のカードの印と、完了数／TODO 数。未来の日は薄く、選べない
  expect(cards[2].textContent?.replace(/\s+/g, " ").trim()).toContain("11.04 Wed Today 1/5");
  expect(cards[3].classList.contains("future")).toBe(true);
  expect(cards[0].classList.contains("future")).toBe(false);
});

test("Daily notes に GOOD / STUCK の1行目を出し（日本語・英語の見出し）、プレビューに数字・振り返り・ノートを出す", async () => {
  await start();
  await openArchive();
  const rows = $$(".archive .entry").map((e) => e.textContent?.replace(/\s+/g, " ").trim());
  expect(rows).toEqual([
    "11.04 Wed Today 1/5 GOOD— STUCK—",
    "11.03 Tue 2/2 GOOD設計を終えた STUCKCI が落ちる",
    "10.30 Fri 1/1 GOODshipped STUCKflaky tests",
  ]);
  expect($(".archive .entry[aria-pressed='true']")?.textContent).toContain("11.03 Tue");
  // プレビュー：選んだ日の数字、振り返りの全文、ノートの時刻付き見出しと最初の行
  expect(text(".archive .prev .ph")).toBe("11.03Tue Open↵");
  expect(text(".archive .prev .ps")).toBe("2/2 Done · 1 Carried · 0 Resolved");
  expect(previewLines()).toBe(
    "Reflection | 良かったこと | 設計を終えた | 詰まったこと | CI が落ちる | Note | 9:30 定例 | 議事1 | 議事2",
  );
});

test("← → で日を移り、Mod+[ / Mod+] で週を移る。今日より先へは行かない", async () => {
  const calls = await start();
  await openArchive();
  const main = $(".archive") as HTMLElement;
  press({ key: "ArrowRight" }, main);
  await until(() => selected() === "11.04 Wed Today 1/5");
  // 今日より先へは移らない（読み直しもしない）
  const before = archiveCalls(calls).length;
  press({ key: "ArrowRight" }, main);
  press({ key: "ArrowRight" }, main);
  expect(selected()).toBe("11.04 Wed Today 1/5");
  expect(archiveCalls(calls).length).toBe(before);
  press({ key: "ArrowLeft" }, main);
  press({ key: "ArrowLeft" }, main);
  await until(() => selected() === "11.02 Mon 3/4");
  // 前の週へ。同じ曜日（月曜）
  press(MOD_LEFT);
  await until(() => text(".archive .range") === "10.26 Mon — 10.30 Fri");
  expect(archiveCalls(calls).at(-1)).toBe("2026-10-26");
  // 次の週へ。同じ曜日へ戻る
  press(MOD_RIGHT);
  await until(() => text(".archive .range") === "11.02 Mon — 11.06 Fri");
  // 今日より先の週へは、今日で止まる
  press(MOD_RIGHT);
  await until(() => archiveCalls(calls).at(-1) === TODAY);
  expect(selected()).toBe("11.04 Wed Today 1/5");
  // 週の移動は、表示している日を動かさない
  expect($("main.content")?.dataset.date).toBe(TODAY);
});

test("週のカードとサイドバーの Recent で日を選ぶ。未来のカードは選べない。Mod+T は今日を選ぶ", async () => {
  const calls = await start();
  await openArchive();
  $$(".archive .card")[0].click();
  await until(() => selected() === "11.02 Mon 3/4");
  const n = archiveCalls(calls).length;
  $$(".archive .card")[4].click();
  expect(archiveCalls(calls).length).toBe(n);
  expect(selected()).toBe("11.02 Mon 3/4");
  // Daily notes の行
  $$(".archive .entry")[2].click();
  await until(() => text(".archive .prev .pd").startsWith("10.30"));
  // サイドバーの Recent も、ふりかえりの中で選ぶ（表示している日は動かさない）
  $$(".recent-btn")[1].click();
  await until(() => selected() === "11.03 Tue 2/2");
  expect($(".recent-btn[aria-current='date']")?.textContent).toContain("11.03");
  expect($("main.content")?.dataset.date).toBe(TODAY);
  press({ key: "t", code: "KeyT", ctrlKey: true });
  await until(() => selected() === "11.04 Wed Today 1/5");
  // Mod+J は受けない
  press({ key: "j", code: "KeyJ", ctrlKey: true });
  expect($("main.content")?.dataset.mode).toBe("board");
});

test("Enter（とダブルクリック・Open）で選んだ日の整理モードを開き、Esc と Mod+Shift+L で元の画面に戻る", async () => {
  await start();
  // ノートモードから開く：Esc で戻ると、ノートモードのまま
  press({ key: "j", code: "KeyJ", ctrlKey: true });
  await until(() => $("main.content")?.dataset.mode === "note");
  await openArchive();
  press({ key: "Escape" }, $(".archive") as HTMLElement);
  await until(() => $(".archive") === null);
  expect($("main.content")?.dataset.mode).toBe("note");
  expect($("main.content")?.dataset.date).toBe(TODAY);
  expect($(".top .datenav")).not.toBeNull();

  // Mod+Shift+L でも閉じる
  await openArchive();
  press(MOD_SHIFT_L);
  await until(() => $(".archive") === null);

  // Enter：選んだ日（11-03）の整理モードを開く。ノートモードから開いても整理モードになる
  await openArchive();
  press({ key: "Enter" }, $(".archive") as HTMLElement);
  await until(() => $(".archive") === null && $("main.content")?.dataset.date === "2026-11-03");
  expect($("main.content")?.dataset.mode).toBe("board");

  // ダブルクリック（週のカード）で、その日を開く
  await openArchive();
  $$(".archive .card")[0].dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  await until(() => $(".archive") === null && $("main.content")?.dataset.date === "2026-11-02");

  // 「Open ↵」
  await openArchive();
  $(".archive .open")!.click();
  await until(() => $(".archive") === null);
  expect($("main.content")?.dataset.date).toBe("2026-11-02");
});

test("ノートや振り返りのエディタにフォーカスがあっても Mod+Shift+L で開き、開いている間は裏の画面のキーが動かない", async () => {
  const calls = await start();
  // 振り返りのエディタ（整理モード）
  const reflection = $(".board .cm-content") as HTMLElement;
  reflection.focus();
  expect(document.activeElement).toBe(reflection);
  press(MOD_SHIFT_L, reflection);
  await until(() => $(".archive .card") !== null);
  expect(document.activeElement).toBe($(".archive"));
  // 裏の整理モードのキー（取り消し・領域の移動・完了の切替）は、ふりかえりの間は動かさない
  const before = calls.length;
  press({ key: "z", code: "KeyZ", ctrlKey: true });
  press({ key: "1", code: "Digit1", ctrlKey: true });
  press({ key: "Enter", ctrlKey: true });
  expect(document.activeElement).toBe($(".archive"));
  expect(calls.slice(before).map((c) => c.cmd)).not.toContain("save_item");
  expect($(".archive")).not.toBeNull();
  // 戻ると、振り返りのエディタへフォーカスが戻る
  press({ key: "Escape" }, $(".archive") as HTMLElement);
  await until(() => $(".archive") === null);
  expect(document.activeElement).toBe(reflection);

  // ノートのエディタ
  press({ key: "j", code: "KeyJ", ctrlKey: true });
  await until(() => $("main.content")?.dataset.mode === "note");
  const note = $(".note .cm-content") as HTMLElement;
  await until(() => document.activeElement === note);
  press(MOD_SHIFT_L, note);
  await until(() => $(".archive .card") !== null);
  // ノートの時刻付き見出しの挿入（Mod+Shift+H）は、ふりかえりの間は動かさない
  press({ key: "H", code: "KeyH", ctrlKey: true, shiftKey: true });
  expect(calls.slice(before).map((c) => c.cmd)).not.toContain("save_text_doc");
  press(MOD_SHIFT_L);
  await until(() => $(".archive") === null);
  await until(() => document.activeElement === note);
});

test("見ている日がある（今日でない）ときは、その日から始める", async () => {
  const calls = await start();
  press(MOD_LEFT);
  await until(() => $("main.content")?.dataset.date === "2026-11-03");
  await openArchive();
  expect(archiveCalls(calls)).toEqual(["2026-11-03"]);
});

test("サイドバーの Archive とパレットのコマンドで開く。開いていれば何もしない。パレットで日を選ぶと閉じる", async () => {
  const calls = await start();
  $$(".side .nav-btn").find((b) => b.textContent?.includes("Archive"))!.click();
  await until(() => $(".archive .card") !== null);
  // 開いている間のサイドバー・パレットの「開く」は、閉じない
  $$(".side .nav-btn").find((b) => b.textContent?.includes("Archive"))!.click();
  flushSync();
  expect($(".archive")).not.toBeNull();
  expect(archiveCalls(calls)).toEqual([null]);

  // パレットの「今日へ移動」（空のとき最初の行）は、ふりかえりの中で今日を選ぶ
  await openPalette();
  expect(paletteSelected()).toBe("今日へ移動 Ctrl+T");
  press({ key: "Enter" }, paletteInput());
  await until(() => $("[data-palette]") === null && selected() === "11.04 Wed Today 1/5");
  expect($(".archive")).not.toBeNull();
  // パレットを閉じたら、ふりかえりの枠にフォーカスが戻り、キーが効く
  expect(document.activeElement).toBe($(".archive"));

  // パレットで日付を選ぶと、ふりかえりを閉じてその日を開く
  await openPalette();
  const input = paletteInput();
  input.value = "2026-11-02";
  input.dispatchEvent(new InputEvent("input", { bubbles: true }));
  await until(() => paletteSelected() === "2026.11.02 Mon を開く");
  press({ key: "Enter" }, input);
  await until(() => $(".archive") === null && $("main.content")?.dataset.date === "2026-11-02");
});

test("サイドバーの Today は、今日のボードを開く", async () => {
  await start();
  press(MOD_LEFT);
  await until(() => $("main.content")?.dataset.date === "2026-11-03");
  await openArchive();
  $$(".side .nav-btn").find((b) => b.textContent?.includes("Today"))!.click();
  await until(() => $(".archive") === null && $("main.content")?.dataset.date === TODAY);
  expect($("main.content")?.dataset.mode).toBe("board");
});

test("データが変わったら読み直す。読めなければ文を出して読み直す", async () => {
  const calls = await start();
  await openArchive();
  const n = archiveCalls(calls).length;
  await emit(EVENTS.changed, { by: "agent" });
  await until(() => archiveCalls(calls).length === n + 1);
  // 読み直しは、いま選んでいる日で行う
  expect(archiveCalls(calls).at(-1)).toBe("2026-11-03");
  // 日付が変わったときも
  await emit(EVENTS.todayChanged, { today: "2026-11-05" });
  await until(() => archiveCalls(calls).length === n + 2);
});

test("開けなかったときは文を出し、読めるまで読み直す", async () => {
  state.failArchive = true;
  const calls = await start();
  press(MOD_SHIFT_L);
  await until(() => $(".archive .none") !== null);
  expect(text(".archive .none")).toBe("ふりかえりを読めませんでした。読み直しています。");
  state.failArchive = false;
  await until(() => $(".archive .card") !== null);
  expect(archiveCalls(calls).length).toBeGreaterThan(1);
});

test("持ち越しの画面を出すときは、ふりかえりを閉じる", async () => {
  await start();
  await openArchive();
  state.carry = { from: "2026-11-03", prompt: "show" };
  window.dispatchEvent(new Event("focus"));
  await until(() => $("[data-carryover]") !== null);
  expect($(".archive")).toBeNull();
  expect($("main.content")?.dataset.mode).toBe("board");
});

test("曜日は英字、振り返りが無い日と記録の無い日は文を出す", async () => {
  expect(weekday("2026-11-04")).toBe("Wed");
  await start();
  await openArchive();
  $$(".archive .card")[0].click();
  await until(() => selected()?.startsWith("11.02") === true);
  expect(previewLines()).toBe("Reflection | 振り返りはありません | Note | ノートはありません");
  $$(".archive .entry")[0].click();
  await until(() => text(".archive .prev .pd").startsWith("11.04"));
});
