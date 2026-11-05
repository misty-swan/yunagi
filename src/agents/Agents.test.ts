// エージェント変更履歴（S-06。画面設計書 8章）：アプリ全体を描き、開く入口・一覧・絞り込み・確認済み・取り消しを
// 確かめる。コアは模擬。
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import App from "../App.svelte";
import type { AgentRow, ChangeEntry, ChangeList, RevertPlan, Settings } from "../lib/bindings";
import { prefs } from "../lib/prefs.svelte";

let app: ReturnType<typeof mount> | undefined;

// jsdom には Range の位置が無い（CodeMirror が測るときに使う）
Range.prototype.getClientRects ??= () => [] as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();

const TODAY = "2026-11-03";

const SETTINGS: Settings = {
  dayBoundary: "04:00",
  meterStart: "09:00",
  meterEnd: "19:00",
  locale: "ja",
  updateCheck: true,
  theme: "system",
  fontSize: 15,
  noteFont: "body",
  layout: "A",
  carryoverReviewThreshold: 3,
  sidebarOpen: true,
  shortcuts: {},
  vimMode: false,
  vimImeRestore: true,
};

const AGENT_ROWS: AgentRow[] = [
  { id: "00000000000000000000000001", name: "cli", scopes: ["read"], createdAt: 1793000000000, lastSeenAt: null, removable: false },
  { id: "00000000000000000000000002", name: "unknown", scopes: ["read"], createdAt: 1793000000000, lastSeenAt: null, removable: false },
  { id: "01J00000000000000000000001", name: "claude-code", scopes: ["read", "todo"], createdAt: 1793000000000, lastSeenAt: 1793600000000, removable: false },
];

function entry(id: string, over: Partial<ChangeEntry> = {}): ChangeEntry {
  return {
    id,
    createdAt: 1793600000000,
    actor: "agent",
    client: "claude-code",
    operation: "todo.edit",
    target: {
      type: "item",
      id: "01J0000000000000000000ITEM",
      item: { date: TODAY, kind: "todo", text: "レート制限を調べる", deleted: false },
    },
    summary: { type: "fields", fields: [{ name: "text", before: "調べる", after: "レート制限を調べる" }] },
    seenAt: null,
    revertedAt: null,
    revertOf: null,
    ...over,
  };
}

const E1 = entry("01J000000000000000000000E1");
const E2 = entry("01J000000000000000000000E2", {
  operation: "todo.complete",
  summary: { type: "fields", fields: [{ name: "status", before: "open", after: "done" }] },
  seenAt: 1793600100000,
});
const E3 = entry("01J000000000000000000000E3", {
  operation: "note.append",
  client: "cli",
  target: { type: "note", date: TODAY },
  summary: { type: "text", beforeChars: 120, afterChars: 180 },
});
const E4 = entry("01J000000000000000000000E4", {
  actor: "human",
  client: null,
  operation: "agent.revert",
  seenAt: 1793600200000,
  revertOf: "01J000000000000000000000E9",
});

const FIELDS_PLAN: RevertPlan = {
  id: E1.id,
  operation: "todo.edit",
  client: "claude-code",
  createdAt: 1793600000000,
  state: "apply",
  token: "token-1",
  changedSince: [],
  effect: {
    type: "fields",
    date: TODAY,
    kind: "todo",
    text: "レート制限を調べる",
    fields: [{ name: "text", current: "レート制限を調べる", restore: "調べる", changedSince: false }],
  },
};

interface Mock {
  calls: { cmd: string; args: Record<string, unknown> }[];
  /** コマンドごとの答え（関数なら呼んで、`kind` を持つ値なら失敗にする）。 */
  answer: Record<string, unknown>;
  unseen: number;
  count: (cmd: string) => number;
  last: (cmd: string) => Record<string, unknown> | undefined;
  order: string[];
}

function mockCore(): Mock {
  const m: Mock = {
    calls: [],
    answer: {},
    unseen: 2,
    count: (cmd) => m.calls.filter((c) => c.cmd === cmd).length,
    last: (cmd) => m.calls.filter((c) => c.cmd === cmd).at(-1)?.args,
    order: [],
  };
  const counts = { todos: 0, done: 0, leftTodos: 0, leftBlockers: 0, resolved: 0, carried: 0 };
  mockIPC(
    async (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      m.calls.push({ cmd, args: a });
      m.order.push(cmd);
      if (cmd in m.answer) {
        const v = m.answer[cmd];
        const r = typeof v === "function" ? (v as (a: Record<string, unknown>) => unknown)(a) : v;
        if (r instanceof Object && "kind" in r) throw r;
        return r;
      }
      switch (cmd) {
        case "plugin:app|version":
          return "0.4.0";
        case "startup_status":
          return { state: "ready" };
        case "today":
          return TODAY;
        case "get_board":
          return { date: a.date, todos: [], blockers: [], reflection: "", note: { headings: [], length: 0 }, reflectionRev: 0, noteRev: 0 };
        case "get_text_doc":
          return { body: "", rev: 0 };
        case "report_ui_state":
        case "carryover_status":
          return null;
        case "set_global_shortcut":
          return { state: "registered", wayland: false };
        case "get_overview":
          return { counts, recent: [{ date: TODAY, counts }], unseenChanges: m.unseen };
        case "get_settings":
          return SETTINGS;
        case "list_agents":
          return AGENT_ROWS;
        case "api_status":
          return { state: "listening", connections: 0 };
        case "list_changes": {
          const page: ChangeList = { changes: [E1, E2, E3, E4], nextBefore: null };
          return page;
        }
        case "mark_changes_seen":
          return (a.ids as string[]).length;
        case "preview_revert":
          return FIELDS_PLAN;
        case "revert_change":
          return { updated: true, state: "apply", marked: true };
      }
    },
    { shouldMockEvents: true },
  );
  return m;
}

beforeEach(() => {
  prefs.apply(SETTINGS);
});

afterEach(async () => {
  if (app) unmount(app);
  app = undefined;
  document.body.innerHTML = "";
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.style.removeProperty("--fs-setting");
  await new Promise((resolve) => setTimeout(resolve, 0));
  clearMocks();
});

async function until(check: () => boolean) {
  await vi.waitFor(() => {
    flushSync();
    expect(check()).toBe(true);
  });
}

const $ = <T extends HTMLElement = HTMLElement>(sel: string) => document.querySelector<T>(sel);
const $$ = (sel: string) => [...document.querySelectorAll<HTMLElement>(sel)];
const text = (sel: string) => $(sel)?.textContent?.replace(/\s+/g, " ").trim() ?? "";
const panel = () => $("[data-agents]");
const row = (id: string) => $(`[data-change="${id}"]`)!;

/** 背後を操作できなくしているか（jsdom は inert の属性を付けないので、プロパティを見る）。 */
function inert(el: Element | null): boolean {
  for (let e = el; e !== null; e = e.parentElement) if ((e as HTMLElement).inert) return true;
  return false;
}

function press(init: KeyboardEventInit, target: EventTarget = window) {
  target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  flushSync();
}

async function start() {
  const core = mockCore();
  app = mount(App, { target: document.body });
  await until(() => $("main.content")?.dataset.date === TODAY && $(".meter") !== null);
  return core;
}

/** サイドバーの Agents から開く。 */
async function openPanel(core: Mock) {
  const nav = $$(".nav-btn").find((b) => b.textContent?.includes("Agents"))!;
  nav.click();
  await until(() => panel() !== null && $$("[data-change]").length > 0);
  return core;
}

function buttonIn(scope: Element, label: string): HTMLButtonElement {
  const b = [...scope.querySelectorAll<HTMLButtonElement>("button")].find((x) => x.textContent?.trim() === label);
  if (b === undefined) throw new Error(`no button: ${label}`);
  return b;
}

test("サイドバーの Agents と上部のバーの「◆ 件数」のどちらからも、同じ変更履歴を開く。開いただけでは確認済みにしない", async () => {
  const core = await start();
  // 上部のバー
  const bar = $$("button").find((b) => b.getAttribute("aria-label") === "エージェントによる未確認の変更 2 件")!;
  bar.click();
  await until(() => panel() !== null && $$("[data-change]").length > 0);
  expect(panel()!.getAttribute("aria-modal")).toBe("true");
  expect(core.last("list_changes")).toEqual({ client: null, unseenOnly: false, before: null, limit: 50 });
  expect(core.count("mark_changes_seen")).toBe(0);
  // 背後は操作できない
  expect(inert($(".content"))).toBe(true);
  expect(inert(panel())).toBe(false);
  press({ key: "Escape" }, document.activeElement!);
  flushSync();
  expect(panel()).toBeNull();
  // サイドバー
  const nav = $$(".nav-btn").find((b) => b.textContent?.includes("Agents"))!;
  nav.click();
  await until(() => panel() !== null);
  expect(core.count("mark_changes_seen")).toBe(0);
});

test("一覧は新しい順に、操作・エージェント・対象・変更内容の要約を出す。未確認には印が付く", async () => {
  const core = await start();
  await openPanel(core);
  expect($$("[data-change]").map((r) => r.dataset.change)).toEqual([E1.id, E2.id, E3.id, E4.id]);
  expect(text(`[data-change="${E1.id}"] .op`)).toBe("TODO を編集");
  expect(text(`[data-change="${E1.id}"] .who`)).toBe("Claude Code");
  expect(text(`[data-change="${E1.id}"] .target`)).toContain("レート制限を調べる");
  expect(text(`[data-change="${E1.id}"] .sum`)).toBe("本文 調べる → レート制限を調べる");
  expect(text(`[data-change="${E2.id}"] .sum`)).toBe("状態 未完了 → 完了");
  expect(text(`[data-change="${E3.id}"] .sum`)).toBe("120 字 → 180 字");
  // 未確認（E1・E3）だけに印とボタンがある。確認済みの E2 と、人の取り消しの記録 E4 には付かない
  expect(row(E1.id).querySelector(".dot")).not.toBeNull();
  expect(row(E3.id).querySelector(".dot")).not.toBeNull();
  expect(row(E2.id).querySelector(".dot")).toBeNull();
  expect(row(E4.id).querySelector(".dot")).toBeNull();
  // 人の取り消しの記録は、取り消せない
  expect(text(`[data-change="${E4.id}"] .op`)).toBe("取り消し");
  expect(row(E4.id).textContent).toContain("あなたが取り消しました");
  expect(() => buttonIn(row(E4.id), "取り消す")).toThrow();
  // 権限の切り替えもここから行える
  expect($$("[data-agent]").map((r) => r.dataset.agent)).toEqual(["cli", "unknown", "claude-code"]);
});

test("確認済みは、行ごとと、表示している未確認すべてに付ける。見せた記録の ID だけを送る", async () => {
  const core = await start();
  await openPanel(core);
  buttonIn(row(E1.id), "確認済みにする").click();
  await until(() => core.count("mark_changes_seen") === 1);
  expect(core.last("mark_changes_seen")).toEqual({ ids: [E1.id] });
  const all = $<HTMLButtonElement>(".mark-all")!;
  expect(all.getAttribute("aria-label")).toBe("表示中の未確認 2 件をすべて確認済みにする");
  all.click();
  await until(() => core.count("mark_changes_seen") === 2);
  // 未確認（E1・E3）だけ。確認済みの E2 と人の記録 E4 は送らない
  expect(core.last("mark_changes_seen")).toEqual({ ids: [E1.id, E3.id] });
});

test("エージェントと「未確認のみ」で絞り込み、続きがあれば「さらに読み込む」で読む", async () => {
  const core = await start();
  core.answer.list_changes = (a: Record<string, unknown>) => ({
    changes: a.before === null ? [E1, E2] : [E3],
    nextBefore: a.before === null ? E2.id : null,
  });
  await openPanel(core);
  expect($$("[data-change]")).toHaveLength(2);
  buttonIn(panel()!, "さらに読み込む").click();
  await until(() => $$("[data-change]").length === 3);
  expect(core.last("list_changes")).toEqual({ client: null, unseenOnly: false, before: E2.id, limit: 50 });
  expect($$("button").some((b) => b.textContent?.trim() === "さらに読み込む")).toBe(false);

  const select = $<HTMLSelectElement>('[data-filter="client"]')!;
  select.value = "claude-code";
  select.dispatchEvent(new Event("change", { bubbles: true }));
  await until(() => core.last("list_changes")?.client === "claude-code");
  const unseen = $<HTMLInputElement>('[data-filter="unseen"]')!;
  unseen.click();
  await until(() => core.last("list_changes")?.unseenOnly === true);
  expect(core.last("list_changes")).toEqual({ client: "claude-code", unseenOnly: true, before: null, limit: 50 });
});

test("取り消しは、取り消す内容を確かめ、確かめた目印を付けて行う。確かめる文は行の下に出し、初めのフォーカスは「やめる」", async () => {
  const core = await start();
  await openPanel(core);
  core.calls.length = 0;
  buttonIn(row(E1.id), "取り消す").click();
  await until(() => row(E1.id).querySelector(".revert table") !== null);
  expect(core.last("preview_revert")).toEqual({ changeId: E1.id });
  const box = row(E1.id).querySelector(".revert")!;
  // 確定のボタンではなく「やめる」にフォーカスがある（Enter の二度押しで確定しない）
  await until(() => document.activeElement?.textContent?.trim() === "やめる");
  expect(box.contains(document.activeElement)).toBe(true);
  expect(box.textContent).toContain("次の欄を、記録の前の値に戻します。");
  expect(box.textContent).toContain("本文");
  expect(box.textContent).toContain("調べる");
  // 人が変えた欄が無いので、通常の「取り消す」だけを出す
  expect(() => buttonIn(box, "上書きして取り消す")).toThrow();
  buttonIn(box, "取り消す").click();
  await until(() => core.count("revert_change") === 1);
  expect(core.last("revert_change")).toEqual({ changeId: E1.id, force: false, expect: "token-1" });
  await until(() => row(E1.id).querySelector(".revert") === null);
  expect(text(".result")).toBe("取り消しました。");
});

test("人が記録のあとに変えた欄があれば、上書きして取り消すだけを出し、見せた値で上書きする", async () => {
  const core = await start();
  core.answer.preview_revert = {
    ...FIELDS_PLAN,
    changedSince: ["text"],
    effect: {
      ...FIELDS_PLAN.effect,
      fields: [{ name: "text", current: "人が直した本文", restore: "調べる", changedSince: true }],
    },
  };
  await openPanel(core);
  buttonIn(row(E1.id), "取り消す").click();
  await until(() => row(E1.id).querySelector(".revert table") !== null);
  const box = row(E1.id).querySelector(".revert")!;
  expect(box.textContent).toContain("記録のあとに「本文」が変更されています。上書きすると、その変更が失われます。");
  expect(box.textContent).toContain("人が直した本文");
  expect(() => buttonIn(box, "取り消す")).toThrow();
  buttonIn(box, "上書きして取り消す").click();
  await until(() => core.count("revert_change") === 1);
  expect(core.last("revert_change")).toEqual({ changeId: E1.id, force: true, expect: "token-1" });
});

test("確かめたあとに内容が変わっていたら、書かずに確かめ直す", async () => {
  const core = await start();
  await openPanel(core);
  buttonIn(row(E1.id), "取り消す").click();
  await until(() => row(E1.id).querySelector(".revert table") !== null);
  core.answer.revert_change = { kind: "conflict", reason: "changed", fields: ["text"], afterPreview: true };
  core.answer.preview_revert = { ...FIELDS_PLAN, token: "token-2" };
  buttonIn(row(E1.id).querySelector(".revert")!, "取り消す").click();
  await until(() => text(".revert .warn").includes("確認したあとに内容が変わりました"));
  expect(core.count("preview_revert")).toBe(2);
  // 新しい目印で行う
  delete core.answer.revert_change;
  buttonIn(row(E1.id).querySelector(".revert")!, "取り消す").click();
  await until(() => core.count("revert_change") === 2);
  expect(core.last("revert_change")).toEqual({ changeId: E1.id, force: false, expect: "token-2" });
});

test("追加の取り消し・ノートの差分を、行の下に出す", async () => {
  const core = await start();
  core.answer.preview_revert = (a: Record<string, unknown>) =>
    a.changeId === E3.id
      ? {
          ...FIELDS_PLAN,
          id: E3.id,
          operation: "note.append",
          effect: { type: "text", doc: "note", date: TODAY, removed: ["追記した行"], added: [], truncated: false },
        }
      : {
          ...FIELDS_PLAN,
          effect: { type: "delete_item", date: TODAY, kind: "todo", text: "足した TODO", carriedToDate: "2026-11-04" },
        };
  await openPanel(core);
  buttonIn(row(E3.id), "取り消す").click();
  await until(() => row(E3.id).querySelector(".revert .diff") !== null);
  expect(text(`[data-change="${E3.id}"] .revert .removed`)).toBe("- 追記した行");
  expect(text(`[data-change="${E3.id}"] .revert`)).toContain("のノートを、記録の前の本文に戻します。");
  buttonIn(row(E3.id), "やめる").click();
  flushSync();
  expect(row(E3.id).querySelector(".revert")).toBeNull();
  buttonIn(row(E1.id), "取り消す").click();
  await until(() => text(`[data-change="${E1.id}"] .revert`).includes("を削除します。"));
  expect(text(`[data-change="${E1.id}"] .revert`)).toContain("に持ち越した項目は残ります。");
});

test("取り消せない理由（持ち越し済み・入力中・保存できていない）は、取り消す内容の代わりに出す", async () => {
  const core = await start();
  core.answer.preview_revert = { kind: "conflict", reason: "carried_over", carriedToId: "01J0", carriedToDate: "2026-11-04" };
  await openPanel(core);
  buttonIn(row(E1.id), "取り消す").click();
  await until(() => row(E1.id).querySelector(".revert .warn") !== null);
  expect(text(`[data-change="${E1.id}"] .revert .warn`)).toContain("持ち越し済みで");
  expect(core.count("revert_change")).toBe(0);
  buttonIn(row(E1.id), "やめる").click();
  flushSync();

  core.answer.preview_revert = FIELDS_PLAN;
  core.answer.revert_change = { kind: "conflict", reason: "user_editing" };
  buttonIn(row(E1.id), "取り消す").click();
  await until(() => row(E1.id).querySelector(".revert table") !== null);
  buttonIn(row(E1.id).querySelector(".revert")!, "取り消す").click();
  await until(() => text(`[data-change="${E1.id}"] .revert .warn`).includes("入力中のため"));
  buttonIn(row(E1.id), "やめる").click();
  flushSync();

  core.answer.revert_change = { kind: "conflict", reason: "save_failing" };
  buttonIn(row(E1.id), "取り消す").click();
  await until(() => row(E1.id).querySelector(".revert table") !== null);
  buttonIn(row(E1.id).querySelector(".revert")!, "取り消す").click();
  await until(() => text(`[data-change="${E1.id}"] .revert .warn`).includes("保存できていない入力がある"));
});

test("Esc は取り消しの確かめる文を先に閉じ、次に変更履歴を閉じて、開く前のフォーカスへ戻る", async () => {
  const core = await start();
  const before = $(".search") as HTMLElement;
  before.focus();
  await openPanel(core);
  buttonIn(row(E1.id), "取り消す").click();
  await until(() => row(E1.id).querySelector(".revert table") !== null);
  press({ key: "Escape" }, document.activeElement!);
  flushSync();
  expect(row(E1.id).querySelector(".revert")).toBeNull();
  expect(panel()).not.toBeNull();
  press({ key: "Escape" }, document.activeElement!);
  flushSync();
  expect(panel()).toBeNull();
  await until(() => document.activeElement !== null);
});

test("変更履歴を開いている間は、ほかの画面のキーを受け付けず、検索パレットも開かない", async () => {
  const core = await start();
  await openPanel(core);
  const palette = new KeyboardEvent("keydown", { key: "k", ctrlKey: true, bubbles: true, cancelable: true });
  document.activeElement!.dispatchEvent(palette);
  flushSync();
  expect($(".palette, [data-palette]")).toBeNull();
  expect(panel()).not.toBeNull();
  expect(core.count("list_changes")).toBe(1);
});
