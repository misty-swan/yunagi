import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import App from "./App.svelte";
import { EVENTS, type Board, type Counts, type Overview } from "./lib/bindings";
import { prefs } from "./lib/prefs.svelte";

let app: ReturnType<typeof mount> | undefined;

beforeEach(() => {
  prefs.lang = "ja";
  clock.today = TODAY;
  setWidth(1280);
});

// テストが失敗しても、描いた画面を必ず片付ける
afterEach(async () => {
  if (app) unmount(app);
  app = undefined;
  document.body.innerHTML = "";
  // 片付けで、コアのイベントを聞くのをやめ終わってから（次のタスクまで待つ）、模擬を外す
  await new Promise((resolve) => setTimeout(resolve, 0));
  clearMocks();
});

function setWidth(width: number) {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: width });
  window.dispatchEvent(new Event("resize"));
}

const TODAY = "2026-11-03";
/** 模擬のコアの今日（テストの中で日付を変えるときに進める）。 */
const clock = { today: TODAY };

function counts(c: Partial<Counts>): Counts {
  return { todos: 0, done: 0, leftTodos: 0, leftBlockers: 0, resolved: 0, carried: 0, ...c };
}

function emptyBoard(date: string): Board {
  return { date, todos: [], blockers: [], reflection: "", note: { headings: [], length: 0 } };
}

/** 模擬のコア（今日は 11-03 Tue。TODO 9 件のうち完了 3、障害 3 件は未解消、持ち越し 4）。 */
function mockCore() {
  const calls: { cmd: string; args: unknown }[] = [];
  const today = counts({ todos: 9, done: 3, leftTodos: 6, leftBlockers: 3, carried: 4 });
  mockIPC(
    (cmd, args) => {
      calls.push({ cmd, args });
      switch (cmd) {
        case "startup_status":
          return { state: "ready" };
        case "today":
          return clock.today;
        case "get_board":
          return emptyBoard((args as { date: string }).date);
        case "get_overview": {
          const date = (args as { date: string }).date;
          const o: Overview = {
            counts: date === TODAY ? today : counts({ todos: 2, done: 2 }),
            recent: [
              { date: clock.today, counts: today },
              { date: "2026-11-02", counts: counts({ todos: 2, done: 2 }) },
            ],
            unseenChanges: 2,
          };
          return o;
        }
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
/** キーの案内の項目（「キー 名前」を ` | ` でつなぐ）。 */
const hints = () =>
  [...document.querySelectorAll(".hints li")].map((li) => li.textContent?.replace(/\s+/g, " ").trim()).join(" | ");
const text = (sel: string) => $(sel)?.textContent?.replace(/\s+/g, " ").trim() ?? "";

function press(init: KeyboardEventInit) {
  window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  flushSync();
}

async function start() {
  const calls = mockCore();
  app = mount(App, { target: document.body });
  await until(() => $("main.content")?.dataset.date === TODAY && $(".meter") !== null);
  return calls;
}

test("コアとつなぎ、開く処理の状態を出す", async () => {
  mockIPC((cmd) => (cmd === "startup_status" ? { state: "opening" } : undefined), {
    shouldMockEvents: true,
  });
  app = mount(App, { target: document.body });
  await until(() => $("main.content")?.dataset.startup === "opening");
});

test("サイドバー・上部のバー・凪メーター・キーの案内に、コアの値を出す", async () => {
  await start();
  // サイドバー：ロゴ、Today の残り件数、Agents の未確認の件数、Recent
  expect(text(".side .word")).toBe("yunagi");
  const nav = [...document.querySelectorAll(".side .nav-btn")].map((b) => b.textContent?.replace(/\s+/g, " ").trim());
  expect(nav).toEqual(["Today 9", "Archive", "Agents 2", "Settings Ctrl+,"]);
  const recent = [...document.querySelectorAll(".recent-btn")].map((b) => b.textContent?.replace(/\s+/g, " ").trim());
  expect(recent).toEqual(["11.03 Tue Today 3/9", "11.02 Mon 2/2"]);
  expect($(".recent-btn")?.getAttribute("aria-current")).toBe("date");
  // すべて終えた日の波は平ら（夕陽色）
  expect(document.querySelectorAll(".recent-btn .wave")[1].classList.contains("calm")).toBe(true);
  expect(text(".side .status")).toBe("Saved locally");

  // 上部のバー：日付、今日の印、Board / Note、検索欄、未確認の変更
  expect(text(".top .date")).toBe("11.03 Tue");
  expect($(".today-pill.is-today")?.tagName).toBe("SPAN");
  expect($(".seg button[aria-pressed='true']")?.textContent?.trim()).toBe("Board");
  expect(text(".search")).toBe("検索・日付へ移動 Ctrl+K");
  expect($(".agent-ind")?.getAttribute("aria-label")).toBe("エージェントによる未確認の変更 2 件");
  // サイドバーを開いているときは、上部のバーにロゴを出さない
  expect($(".top-logo")).toBeNull();

  // 凪メーター：残り、内訳、読み上げ用のまとめ
  expect(text(".meter .num")).toBe("9");
  expect(text(".meter .sub")).toBe("TODO 6 · Blockers 3 · Done 3/9 · Carried 4");
  expect($(".meter")?.getAttribute("aria-label")).toBe("残り 9 件（TODO 6・障害 3）、完了 3/9、持ち越し 4");

  // キーの案内（整理モード）
  expect(hints()).toBe(
    "Ctrl+K Search | Ctrl+J Note | Ctrl+Enter Done | Ctrl+[ Ctrl+] Day | Ctrl+Shift+L Archive | Ctrl+\\ Sidebar",
  );
});

test("日付を ‹ › と Today と Mod+[ / Mod+] / Mod+T で移る", async () => {
  const calls = await start();
  $(".icon-btn")?.click();
  await until(() => $("main.content")?.dataset.date === "2026-11-02");
  // ほかの日を見ているときは、Today は押せるボタンになる。過去の日の凪メーターは Day end
  expect($(".today-pill.go")?.tagName).toBe("BUTTON");
  expect(text(".today-pill.go")).toBe("Today Ctrl+T");
  expect(text(".meter .time")).toBe("Day end");
  expect(text(".meter .num")).toBe("NAGI");
  expect(text(".meter .sub")).toBe("Done 2/2");
  expect(document.querySelectorAll(".recent-btn")[1].getAttribute("aria-current")).toBe("date");

  press({ key: "[", code: "BracketLeft", ctrlKey: true });
  await until(() => $("main.content")?.dataset.date === "2026-11-01");
  press({ key: "t", code: "KeyT", ctrlKey: true });
  await until(() => $("main.content")?.dataset.date === TODAY);
  press({ key: "]", code: "BracketRight", ctrlKey: true });
  await until(() => $("main.content")?.dataset.date === "2026-11-04");
  expect(text(".meter .time")).toBe("Not yet");
  $(".today-pill.go")?.click();
  await until(() => $("main.content")?.dataset.date === TODAY);
  // Recent から選ぶ
  (document.querySelectorAll(".recent-btn")[1] as HTMLElement).click();
  await until(() => $("main.content")?.dataset.date === "2026-11-02");
  expect(calls.filter((c) => c.cmd === "get_board").map((c) => (c.args as { date: string }).date)).toEqual([
    TODAY,
    "2026-11-02",
    "2026-11-01",
    TODAY,
    "2026-11-04",
    TODAY,
    "2026-11-02",
  ]);
});

test("Mod+\\ で開閉し、ノートモードでは閉じて、整理モードに戻ると元に戻す", async () => {
  await start();
  press({ key: "\\", code: "Backslash", ctrlKey: true });
  expect($(".side")).toBeNull();
  expect(text(".top-logo")).toBe("yunagi");
  press({ key: "\\", code: "Backslash", ctrlKey: true });
  expect($(".side")).not.toBeNull();

  // Mod+J でノートモード：サイドバーを閉じ、凪メーターは小さい版、キーの案内も変わる
  press({ key: "j", code: "KeyJ", ctrlKey: true });
  expect($("main.content")?.dataset.mode).toBe("note");
  expect($(".side")).toBeNull();
  expect($(".meter")?.classList.contains("compact")).toBe(true);
  expect(text(".meter .info")).toMatch(/^\d+:\d\d 9 left$/);
  expect(hints()).toBe(
    "Ctrl+Shift+T To TODO | Ctrl+Shift+B To Blockers | Ctrl+Shift+H Time heading | Ctrl+J Board | Ctrl+\\ Sidebar",
  );
  // Board / Note の切り替えでも戻れる
  ($(".seg button") as HTMLElement).click();
  flushSync();
  expect($("main.content")?.dataset.mode).toBe("board");
  expect($(".side")).not.toBeNull();
});

test("ウィンドウ幅 900px 未満では閉じ、Mod+\\ で明示的に開いたときだけ出す", async () => {
  await start();
  setWidth(820);
  flushSync();
  expect($(".side")).toBeNull();
  press({ key: "\\", code: "Backslash", ctrlKey: true });
  expect($(".side")).not.toBeNull();
  setWidth(1280);
  flushSync();
  expect($(".side")).not.toBeNull();
});

test("サイドバーの Today は、今日の整理モードを開く", async () => {
  await start();
  press({ key: "[", code: "BracketLeft", ctrlKey: true });
  await until(() => $("main.content")?.dataset.date === "2026-11-02");
  ($(".seg button:nth-child(2)") as HTMLElement).click();
  flushSync();
  press({ key: "\\", code: "Backslash", ctrlKey: true });
  ($(".side .nav-btn") as HTMLElement).click();
  await until(() => $("main.content")?.dataset.date === TODAY);
  expect($("main.content")?.dataset.mode).toBe("board");
});

test("言語を変えると文が切り替わり、ラベルは英語のまま", async () => {
  await start();
  prefs.lang = "en";
  flushSync();
  expect(text(".search")).toBe("Search or jump to a date Ctrl+K");
  expect($(".meter")?.getAttribute("aria-label")).toBe("9 left (TODOs 6, blockers 3), 3 of 9 done, 4 carried over");
  expect(document.documentElement.lang).toBe("en");
  expect(hints()).toContain("Ctrl+K Search |");
});

test("コアのデータが変わったら、件数を読み直す", async () => {
  const calls = await start();
  const before = calls.filter((c) => c.cmd === "get_overview").length;
  await emit(EVENTS.changed, { by: "agent" });
  await until(() => calls.filter((c) => c.cmd === "get_overview").length === before + 1);
});

test("押し続けたときの繰り返しでは動かさない", async () => {
  await start();
  press({ key: "j", code: "KeyJ", ctrlKey: true });
  press({ key: "j", code: "KeyJ", ctrlKey: true, repeat: true });
  press({ key: "j", code: "KeyJ", ctrlKey: true, repeat: true });
  expect($("main.content")?.dataset.mode).toBe("note");
});

test("ほかの日を見ている間に日付が変わったら、その日のまま。Mod+T で新しい今日へ移る", async () => {
  await start();
  press({ key: "[", code: "BracketLeft", ctrlKey: true });
  await until(() => $("main.content")?.dataset.date === "2026-11-02");
  clock.today = "2026-11-04";
  await emit(EVENTS.todayChanged, { today: "2026-11-04" });
  // すぐに Mod+T を押しても、前の今日ではなく新しい今日へ移る
  press({ key: "t", code: "KeyT", ctrlKey: true });
  await until(() => $("main.content")?.dataset.date === "2026-11-04");
  expect($(".today-pill.is-today")).not.toBeNull();
});

test("読んでいる途中に続けて押しても、1日ずつ進む", async () => {
  await start();
  press({ key: "]", code: "BracketRight", ctrlKey: true });
  press({ key: "]", code: "BracketRight", ctrlKey: true });
  await until(() => $("main.content")?.dataset.date === "2026-11-05");
});
