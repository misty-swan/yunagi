// 設定（S-05。画面設計書 7章）：アプリ全体を描き、設定の開閉・値の保存・起動時の読み込み・データ区分の
// 操作（保存先・バックアップ・書き出し）を確かめる。コアは模擬（状態を持つ）。
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import App from "../App.svelte";
import type { BackupEntry, Settings } from "../lib/bindings";
import { prefs } from "../lib/prefs.svelte";

// OS のフォルダ選択（Tauri のダイアログ）と、画面の読み込み直しは、テストで差し替える
const picked = vi.hoisted(() => ({ values: [] as (string | null)[], asked: [] as string[], reloads: 0 }));
vi.mock("../lib/folder", () => ({
  pickFolder: async (title: string) => {
    picked.asked.push(title);
    return picked.values.shift() ?? null;
  },
}));
vi.mock("../lib/page", () => ({
  reloadPage: () => {
    picked.reloads++;
  },
}));

let app: ReturnType<typeof mount> | undefined;

// jsdom には Range の位置が無い（CodeMirror が測るときに使う）
Range.prototype.getClientRects ??= () => [] as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();

const TODAY = "2026-11-03";

const DEFAULTS: Settings = {
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

const BACKUPS: BackupEntry[] = [
  {
    path: "/data/backups/daily-2026-11-02-1793600000.db",
    kind: { type: "daily", date: "2026-11-02" },
    createdAt: 1793600000000,
  },
  {
    path: "/data/backups/pre-migration-v2-to-v3-1793500000.db",
    kind: { type: "pre_migration", from: 2, to: 3 },
    createdAt: 1793500000000,
  },
];

beforeEach(() => {
  picked.values = [];
  picked.asked = [];
  picked.reloads = 0;
  prefs.apply(DEFAULTS);
});

afterEach(async () => {
  if (app) unmount(app);
  app = undefined;
  document.body.innerHTML = "";
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.style.removeProperty("--fs-setting");
  vi.useRealTimers();
  await new Promise((resolve) => setTimeout(resolve, 0));
  clearMocks();
});

interface Mock {
  calls: { cmd: string; args: Record<string, unknown> }[];
  settings: Settings;
  /** あれば、`get_settings` の答えをこれが解決するまで待たせる。 */
  gate: Promise<void> | null;
  /** 次の `save_settings` を断る。 */
  rejectSave: unknown;
  /** コマンドごとの答えの差し替え（`undefined` なら既定）。 */
  answer: Record<string, unknown>;
  count: (cmd: string) => number;
  last: (cmd: string) => Record<string, unknown> | undefined;
}

function mockCore(settings: Partial<Settings> = {}): Mock {
  const m: Mock = {
    calls: [],
    settings: { ...DEFAULTS, ...settings },
    gate: null,
    rejectSave: undefined,
    answer: {},
    count: (cmd) => m.calls.filter((c) => c.cmd === cmd).length,
    last: (cmd) => m.calls.filter((c) => c.cmd === cmd).at(-1)?.args,
  };
  const counts = { todos: 0, done: 0, leftTodos: 0, leftBlockers: 0, resolved: 0, carried: 0 };
  mockIPC(
    async (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      m.calls.push({ cmd, args: a });
      if (cmd in m.answer) {
        const v = m.answer[cmd];
        if (v instanceof Object && "kind" in v) throw v;
        return v;
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
          return { state: a.binding === null ? "off" : "registered", wayland: false };
        case "get_overview":
          return { counts, recent: [{ date: TODAY, counts }], unseenChanges: 0 };
        case "get_settings":
          if (m.gate !== null) await m.gate;
          return m.settings;
        case "save_settings": {
          if (m.rejectSave !== undefined) {
            const e = m.rejectSave;
            m.rejectSave = undefined;
            throw e;
          }
          const p = a.patch as Partial<Settings>;
          m.settings = { ...m.settings, ...p };
          return m.settings;
        }
        case "get_data_dir":
          return "/data";
        case "list_backups":
          return BACKUPS;
        case "check_data_dir":
          return null;
        case "change_data_dir":
          return { leftovers: [] };
        case "use_data_dir":
        case "restore_backup":
          return null;
        case "export_markdown":
          return ["/out/2026-11-02.md", "/out/2026-11-03.md"];
      }
    },
    { shouldMockEvents: true },
  );
  return m;
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
const dialog = () => $("[data-settings]");
/** 背後を操作できなくしているか（jsdom は inert の属性を付けないので、プロパティを見る）。 */
function inert(el: Element | null): boolean {
  for (let e = el; e !== null; e = e.parentElement) if ((e as HTMLElement).inert) return true;
  return false;
}

function press(init: KeyboardEventInit, target: EventTarget = window) {
  target.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  flushSync();
}

const MOD_COMMA = { key: ",", code: "Comma", ctrlKey: true };

async function start(settings: Partial<Settings> = {}) {
  const core = mockCore(settings);
  app = mount(App, { target: document.body });
  await until(() => $("main.content")?.dataset.date === TODAY && $(".meter") !== null);
  return core;
}

async function open(section?: string) {
  press(MOD_COMMA);
  expect(dialog()).not.toBeNull();
  if (section !== undefined) {
    $(`[data-section="${section}"]`)?.click();
    await until(() => $(`[data-pane="${section}"]`) !== null);
  }
}

/** 文字で探したボタン。 */
function button(label: string): HTMLButtonElement {
  const b = $$("[data-settings] button").find((x) => x.textContent?.trim() === label);
  if (b === undefined) throw new Error(`no button: ${label}`);
  return b as HTMLButtonElement;
}

function change(el: HTMLElement | null, value: string) {
  const input = el as HTMLInputElement | HTMLSelectElement;
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  input.dispatchEvent(new Event("change", { bubbles: true }));
  flushSync();
}

test("Mod+, で開いて 5 つの区分を出し、Esc か Mod+, で閉じて、開く前のフォーカスへ戻る。背後は操作できない", async () => {
  await start();
  const before = $(".search") as HTMLElement;
  before.focus();
  press(MOD_COMMA);
  const d = dialog()!;
  expect(d.getAttribute("aria-modal")).toBe("true");
  expect($$("[data-section]").map((b) => b.textContent?.trim())).toEqual([
    "General",
    "Display",
    "Operation",
    "Data",
    "Agents",
  ]);
  // 背後（外枠と本文）は操作できない
  expect(inert($(".shell"))).toBe(true);
  await until(() => document.activeElement?.getAttribute("data-section") === "general");

  press({ key: "Escape" }, document.activeElement!);
  expect(dialog()).toBeNull();
  expect(inert($(".shell"))).toBe(false);
  await until(() => document.activeElement === before);

  press(MOD_COMMA);
  expect(dialog()).not.toBeNull();
  await until(() => document.activeElement?.getAttribute("data-section") === "general");
  press(MOD_COMMA, document.activeElement!);
  expect(dialog()).toBeNull();
});

test("サイドバーの Settings と、パレットの「設定を開く」からも開く。開いている間は骨組みのキーを受けない", async () => {
  await start();
  // サイドバー
  $$(".side .nav-btn").find((b) => b.textContent?.includes("Settings"))?.click();
  flushSync();
  expect(dialog()).not.toBeNull();
  // Mod+J などの骨組みのキーは効かない（モードは変わらない）
  press({ key: "j", code: "KeyJ", ctrlKey: true }, document.activeElement!);
  expect($("main.content")?.dataset.mode).toBe("board");
  press({ key: "Escape" }, document.activeElement!);
  expect(dialog()).toBeNull();

  // パレット
  press({ key: "k", code: "KeyK", ctrlKey: true });
  await until(() => $("[data-palette]") !== null);
  const input = $("[data-palette] input") as HTMLInputElement;
  input.value = "設定";
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await until(() => $$("[data-palette-row]").some((r) => r.textContent?.includes("設定を開く")));
  $$("[data-palette-row]").find((r) => r.textContent?.includes("設定を開く"))?.click();
  await until(() => dialog() !== null && $("[data-palette]") === null);
});

test("区分ごとに設定を変える：変えた欄だけを保存し、画面にはすぐ当てる", async () => {
  const core = await start();
  await open();

  // 一般
  change($("#set-day-boundary"), "05:30");
  change($("#set-meter-start"), "08:30");
  change($("#set-meter-end"), "18:00");
  change($("#set-lang"), "en");
  const check = $("#set-update") as HTMLInputElement;
  check.checked = false;
  check.dispatchEvent(new Event("change", { bubbles: true }));
  await until(() => core.count("save_settings") === 5);
  expect(core.calls.filter((c) => c.cmd === "save_settings").map((c) => c.args.patch)).toEqual([
    { dayBoundary: "05:30" },
    { meterStart: "08:30" },
    { meterEnd: "18:00" },
    { locale: "en" },
    { updateCheck: false },
  ]);
  expect(prefs.lang).toBe("en");
  expect(document.documentElement.lang).toBe("en");
  expect(prefs.meterHours).toEqual({ start: 8 * 60 + 30, end: 18 * 60 });
  // 言語を変えると、開いている画面の文言がその場で変わる（区分のラベルは英語のまま）
  expect(text('[data-pane="general"]')).toContain("Day boundary");

  // 表示
  $('[data-section="display"]')?.click();
  await until(() => $('[data-pane="display"]') !== null);
  change($("#set-theme"), "dark");
  expect(document.documentElement.dataset.theme).toBe("dark");
  change($("#set-font-size"), "18");
  expect(document.documentElement.style.getPropertyValue("--fs-setting")).toBe("18");
  change($("#set-note-font"), "mono");
  expect(prefs.noteFont).toBe("mono");
  change($("#set-layout"), "B");
  await until(() => $(".board")?.dataset.layout === "B");
  await until(() => core.count("save_settings") === 9);
  expect(core.settings).toMatchObject({ theme: "dark", fontSize: 18, noteFont: "mono", layout: "B" });

  // 操作：持ち越し見直しの回数。1 未満・整数でない値は保存せず、欄を前の値に戻す
  $('[data-section="operation"]')?.click();
  await until(() => $('[data-pane="operation"]') !== null);
  change($("#set-threshold"), "5");
  await until(() => core.settings.carryoverReviewThreshold === 5);
  expect(prefs.reviewThreshold).toBe(5);
  change($("#set-threshold"), "0");
  expect(($("#set-threshold") as HTMLInputElement).value).toBe("5");
  expect(core.count("save_settings")).toBe(10);

  // Vim モード：保存する。入力状態を戻す欄は、Vim モードが有効なときだけ出る
  expect($("#set-vim-ime-restore")).toBeNull();
  const vim = $("#set-vim") as HTMLInputElement;
  vim.checked = true;
  vim.dispatchEvent(new Event("change", { bubbles: true }));
  await until(() => core.settings.vimMode);
  expect(prefs.vimMode).toBe(true);
  await until(() => $("#set-vim-ime-restore") !== null);
  const restore = $("#set-vim-ime-restore") as HTMLInputElement;
  expect(restore.checked).toBe(true);
  restore.checked = false;
  restore.dispatchEvent(new Event("change", { bubbles: true }));
  await until(() => core.settings.vimImeRestore === false);
  expect(prefs.vimImeRestore).toBe(false);
});

test("IME を切り替えられない環境（Linux で fcitx5 も IBus も動いていない）では、Vim モードの欄に制約を出す", async () => {
  const core = await start({ vimMode: true });
  core.answer.ime_status = { available: false, method: null };
  await open("operation");
  await until(() => $(".ime-unavailable") !== null);
  expect(text(".ime-unavailable")).toContain("自動で切り替えられません");
  expect(core.count("ime_status")).toBe(1);
  // 切り替えられる環境では出さない（開き直したとき、問い合わせ直す）
  press({ key: "Escape", code: "Escape" }, $("[data-settings]")!);
  await until(() => dialog() === null);
  core.answer.ime_status = { available: true, method: "fcitx5" };
  await open("operation");
  await until(() => core.count("ime_status") === 2);
  await until(() => $(".ime-unavailable") === null);
  // 問い合わせに失敗したときは、切り替えられないと決めつけない
  press({ key: "Escape", code: "Escape" }, $("[data-settings]")!);
  await until(() => dialog() === null);
  core.answer.ime_status = { kind: "internal", detail: "x" };
  await open("operation");
  await until(() => core.count("ime_status") === 3);
  expect($(".ime-unavailable")).toBeNull();
});

/** 設定の「操作」区分で、操作 `id` の割当のボタン。 */
const keyButton = (id: string) => $(`[data-pane="operation"] [data-action="${id}"] .key-btn`) as HTMLButtonElement;

test("ショートカット割当：キーを押して変えると保存し、骨組みのキーと案内にすぐ当たる。初期割当と同じにすると持たない", async () => {
  const core = await start();
  await open("operation");
  expect(keyButton("palette").textContent?.trim()).toBe("Ctrl+K");

  keyButton("palette").click();
  flushSync();
  expect(keyButton("palette").textContent).toContain("新しいキーを押してください");
  // 修飾キーだけの押下は待ち続ける。取り込み中のキーは、ほかの画面の操作にならない（Ctrl+J でモードが変わらない）
  press({ key: "Control", code: "ControlLeft", ctrlKey: true }, keyButton("palette"));
  expect(keyButton("palette").textContent).toContain("新しいキーを押してください");
  press({ key: "p", code: "KeyP", ctrlKey: true }, keyButton("palette"));
  await until(() => core.settings.shortcuts.palette === "Mod+P");
  expect(core.last("save_settings")).toEqual({ patch: { shortcuts: { palette: "Mod+P" } } });
  expect(keyButton("palette").textContent?.trim()).toBe("Ctrl+P");
  expect($('[data-pane="operation"] [data-action="palette"] .btn')).not.toBeNull();

  // 閉じると、新しい割当で動く（古い割当では動かない）
  press({ key: "Escape" }, document.activeElement!);
  expect(dialog()).toBeNull();
  press({ key: "k", code: "KeyK", ctrlKey: true });
  expect($("[data-palette]")).toBeNull();
  press({ key: "p", code: "KeyP", ctrlKey: true });
  await until(() => $("[data-palette]") !== null);
  // 案内にも新しい割当が出る
  expect($(".search kbd")?.textContent).toBe("Ctrl+P");
  press({ key: "Escape" }, document.activeElement!);
  await until(() => $("[data-palette]") === null);

  // 初期割当と同じキーに戻すと、割当を持たない
  await open("operation");
  keyButton("palette").click();
  flushSync();
  press({ key: "k", code: "KeyK", ctrlKey: true }, keyButton("palette"));
  await until(() => Object.keys(core.settings.shortcuts).length === 0);
  expect(core.last("save_settings")).toEqual({ patch: { shortcuts: {} } });
});

test("ショートカット割当：ほかの操作と同じキーと、割り当てられないキーは、何も変えずに理由を出す。Esc で取り消す", async () => {
  const core = await start();
  await open("operation");
  const before = core.count("save_settings");
  const tryKey = (id: string, init: KeyboardEventInit) => {
    keyButton(id).click();
    flushSync();
    press(init, keyButton(id));
  };
  // 検索パレットに、モードの切替（Mod+J）のキー
  tryKey("palette", { key: "j", code: "KeyJ", ctrlKey: true });
  expect(text('[data-action="palette"] .bad')).toBe("「ノートモード ⇄ 整理モード」と同じキーです。先にそちらを変えてください。");
  // 修飾キーの無いキー、取り消し・やり直しのキー、Ctrl+Alt
  tryKey("palette", { key: "a", code: "KeyA" });
  expect(text('[data-action="palette"] .bad')).toContain("Mod か Alt と組み合わせた");
  tryKey("palette", { key: "z", code: "KeyZ", ctrlKey: true });
  expect(text('[data-action="palette"] .bad')).toContain("ほかの操作が使う");
  tryKey("palette", { key: "[", code: "BracketLeft", ctrlKey: true, altKey: true });
  expect(text('[data-action="palette"] .bad')).toContain("Ctrl と Alt を同時に");
  // 整理モードの操作とノートモードの操作は重ならない（同じキーにできる）。共通の操作とは重なる
  tryKey("toTodo", { key: "k", code: "KeyK", ctrlKey: true });
  expect(text('[data-action="toTodo"] .bad')).toContain("「検索パレット」");
  expect(core.count("save_settings")).toBe(before);

  // 取り込み中の Esc は、取り消すだけで設定を閉じない
  keyButton("today").click();
  flushSync();
  press({ key: "Escape" }, keyButton("today"));
  expect(dialog()).not.toBeNull();
  expect(keyButton("today").textContent?.trim()).toBe("Ctrl+T");
});

test("ショートカット割当：初期値に戻す。保存した割当が使えないときは、初期割当にして知らせる", async () => {
  const core = await start({ shortcuts: { today: "Mod+Q", sidebar: "Mod+Alt+B", palette: "Mod+J" } });
  await open("operation");
  // Mod+Q は使える割当。Ctrl+Alt は使えないので初期割当。Mod+J は先に初期割当のモードの切替が使っているが、
  // 変えた割当が勝ち、初期割当の側が使えなくなる
  expect(keyButton("today").textContent?.trim()).toBe("Ctrl+Q");
  expect(keyButton("sidebar").textContent?.trim()).toBe("Ctrl+\\");
  expect(text('[data-action="sidebar"] .bad')).toBe("設定の割当が使えないキーなので、初期割当にしています。");
  expect(keyButton("palette").textContent?.trim()).toBe("Ctrl+J");
  expect(keyButton("toggleMode").textContent?.trim()).toBe("—");
  expect(text('[data-action="toggleMode"] .bad')).toBe("「検索パレット」に割り当てたキーと重なっているため、いまは使えません。");

  $('[data-action="palette"] .btn')?.click();
  await until(() => core.settings.shortcuts.palette === undefined);
  expect(keyButton("toggleMode").textContent?.trim()).toBe("Ctrl+J");
  button("すべて初期値に戻す").click();
  await until(() => Object.keys(core.settings.shortcuts).length === 0);
  expect(keyButton("today").textContent?.trim()).toBe("Ctrl+T");
});

/** OS 全体のショートカットを OS に頼んだ割当の履歴。 */
const registered = (core: Mock) => core.calls.filter((c) => c.cmd === "set_global_shortcut").map((c) => c.args.binding);

test("OS 全体のショートカット：設定を読んだら登録し、割当を変えるたびに登録し直す。初期値に戻すと初期割当で登録する", async () => {
  const core = await start();
  await until(() => registered(core).length === 1);
  expect(registered(core)).toEqual(["Mod+Shift+D"]);
  await open("operation");
  expect(keyButton("bringToFront").textContent?.trim()).toBe("Ctrl+Shift+D");
  // 登録できていれば、理由は出ない（Wayland でもない）
  expect($('[data-action="bringToFront"] .bad')).toBeNull();

  keyButton("bringToFront").click();
  flushSync();
  press({ key: "E", code: "KeyE", ctrlKey: true, shiftKey: true }, keyButton("bringToFront"));
  await until(() => registered(core).length === 2);
  expect(registered(core)).toEqual(["Mod+Shift+D", "Mod+Shift+E"]);
  expect(core.settings.shortcuts).toEqual({ bringToFront: "Mod+Shift+E" });

  // すべて初期値に戻すと、初期割当で登録し直す
  button("すべて初期値に戻す").click();
  await until(() => registered(core).length === 3);
  expect(registered(core).at(-1)).toBe("Mod+Shift+D");
  expect(core.settings.shortcuts).toEqual({});
});

test("OS 全体のショートカット：ほかの操作と同じキー・英字と数字以外のキーは断り、登録し直さない。同じキーを使う操作があれば登録を外す", async () => {
  const core = await start();
  await until(() => registered(core).length === 1);
  await open("operation");
  const tryKey = (id: string, init: KeyboardEventInit) => {
    keyButton(id).click();
    flushSync();
    press(init, keyButton(id));
  };
  // 共通の操作のキー（OS 全体のキーは、すべての範囲と重なる）
  tryKey("bringToFront", { key: "k", code: "KeyK", ctrlKey: true });
  expect(text('[data-action="bringToFront"] .bad')).toContain("「検索パレット」");
  // 記号のキーと名前のキー
  tryKey("bringToFront", { key: "[", code: "BracketLeft", ctrlKey: true });
  expect(text('[data-action="bringToFront"] .bad')).toContain("英字か数字");
  tryKey("bringToFront", { key: "Enter", code: "Enter", ctrlKey: true });
  expect(text('[data-action="bringToFront"] .bad')).toContain("英字か数字");
  // 整理・ノートの操作も、OS 全体のキー（Mod+Shift+D）は使えない
  tryKey("toTodo", { key: "D", code: "KeyD", ctrlKey: true, shiftKey: true });
  expect(text('[data-action="toTodo"] .bad')).toContain("「Yunagi を前面に呼び出す」");
  expect(core.count("save_settings")).toBe(0);
  expect(registered(core)).toEqual(["Mod+Shift+D"]);
});

test("OS 全体のショートカット：登録できなかった理由と、Wayland の案内を設定に出す。登録できなくても保存は断らない", async () => {
  const core = await start();
  await until(() => registered(core).length === 1);
  await open("operation");
  const change = (init: KeyboardEventInit) => {
    keyButton("bringToFront").click();
    flushSync();
    press(init, keyButton("bringToFront"));
  };

  // ほかのアプリが使っている
  core.answer.set_global_shortcut = { state: "taken", wayland: false };
  change({ key: "E", code: "KeyE", ctrlKey: true, shiftKey: true });
  await until(() => $('[data-action="bringToFront"] .bad') !== null);
  expect(text('[data-action="bringToFront"] .bad')).toContain("ほかのアプリがこのキーを使っている");
  expect(core.settings.shortcuts).toEqual({ bringToFront: "Mod+Shift+E" });

  // そのほかの理由で登録できない
  core.answer.set_global_shortcut = { state: "failed", wayland: false };
  change({ key: "F", code: "KeyF", ctrlKey: true, shiftKey: true });
  await until(() => text('[data-action="bringToFront"] .bad').includes("OS に登録できませんでした"));

  // 部品が使えない
  core.answer.set_global_shortcut = { state: "unavailable", wayland: false };
  change({ key: "G", code: "KeyG", ctrlKey: true, shiftKey: true });
  await until(() => text('[data-action="bringToFront"] .bad').includes("使えません"));

  // Wayland：登録できても案内を出す
  core.answer.set_global_shortcut = { state: "registered", wayland: true };
  change({ key: "Y", code: "KeyY", ctrlKey: true, shiftKey: true });
  await until(() => text('[data-action="bringToFront"] .bad').includes("Wayland"));
  expect(text('[data-action="bringToFront"] .bad')).toContain("yunagi show");
  expect(text('[data-action="bringToFront"] .bad')).not.toContain("ほかのアプリ");
  // 登録できて Wayland でもなければ、出さない
  core.answer.set_global_shortcut = { state: "registered", wayland: false };
  change({ key: "J", code: "KeyJ", ctrlKey: true, shiftKey: true });
  await until(() => $('[data-action="bringToFront"] .bad') === null);
});

test("OS 全体のショートカット：登録に失敗したら、設定を開き直したときに頼み直す。使用中のときは頼み直さない", async () => {
  const core = await start();
  await until(() => registered(core).length === 1);
  await open("operation");
  core.answer.set_global_shortcut = { state: "failed", wayland: false };
  keyButton("bringToFront").click();
  flushSync();
  press({ key: "E", code: "KeyE", ctrlKey: true, shiftKey: true }, keyButton("bringToFront"));
  await until(() => registered(core).length === 2);
  await until(() => $('[data-action="bringToFront"] .bad') !== null);
  press({ key: "Escape", code: "Escape" }, $("[data-settings]")!);
  await until(() => dialog() === null);
  core.answer.set_global_shortcut = { state: "taken", wayland: false };
  await open("operation");
  await until(() => registered(core).length === 3);
  expect(registered(core).at(-1)).toBe("Mod+Shift+E");
  // 使用中は、開き直しても頼み直さない
  await until(() => $('[data-action="bringToFront"] .bad') !== null);
  press({ key: "Escape", code: "Escape" }, $("[data-settings]")!);
  await until(() => dialog() === null);
  await open("operation");
  flushSync();
  expect(registered(core).length).toBe(3);
});

test("OS 全体のショートカット：保存に失敗して割当が元に戻ったら、元のキーで登録し直す", async () => {
  const core = await start();
  await until(() => registered(core).length === 1);
  await open("operation");
  core.rejectSave = { kind: "internal", detail: "x" };
  keyButton("bringToFront").click();
  flushSync();
  press({ key: "E", code: "KeyE", ctrlKey: true, shiftKey: true }, keyButton("bringToFront"));
  // 画面には新しい割当をすぐ当てて登録し、保存に失敗して戻ったら、元の割当で登録し直す
  await until(() => registered(core).length === 3);
  expect(registered(core)).toEqual(["Mod+Shift+D", "Mod+Shift+E", "Mod+Shift+D"]);
  expect(core.settings.shortcuts).toEqual({});
});

test("OS 全体のショートカット：同じキーを使う操作に取られたときは登録を外す", async () => {
  const core = await start({ shortcuts: { palette: "Mod+Shift+D" } });
  // 変えた割当が初期割当に勝ち、「前面に呼び出す」は使えなくなる
  await until(() => registered(core).length === 1);
  expect(registered(core)).toEqual([null]);
  await open("operation");
  expect(text('[data-action="bringToFront"] .bad')).toContain("「検索パレット」に割り当てたキーと重なっている");
});

test("Vim モードの Windows・Linux は、前日・翌日の移動の初期割当が Alt+[ / Alt+] になる。変えた割当は残る", async () => {
  const core = await start({ vimMode: true });
  await open("operation");
  expect(keyButton("prevDay").textContent?.trim()).toBe("Alt+[");
  expect(keyButton("nextDay").textContent?.trim()).toBe("Alt+]");
  expect(text('[data-pane="operation"]')).toContain("Ctrl+[ が Vim の Esc と重なる");
  // 初期割当と同じ Alt+[ にしても、割当は持たない（Vim モードを切ったとき初期割当に従うため）
  keyButton("prevDay").click();
  flushSync();
  press({ key: "[", code: "BracketLeft", altKey: true }, keyButton("prevDay"));
  await until(() => core.count("save_settings") === 1);
  expect(core.settings.shortcuts).toEqual({});
  // Vim モード中は Ctrl+[ を割り当てられない（Esc と重なる）
  keyButton("nextDay").click();
  flushSync();
  press({ key: "[", code: "BracketLeft", ctrlKey: true }, keyButton("nextDay"));
  expect(text('[data-action="nextDay"] .bad')).toContain("ほかの操作が使う");
  expect(core.count("save_settings")).toBe(1);
  // Vim モードを切ると、初期割当に戻る
  const vim = $("#set-vim") as HTMLInputElement;
  vim.checked = false;
  vim.dispatchEvent(new Event("change", { bubbles: true }));
  await until(() => keyButton("prevDay").textContent?.trim() === "Ctrl+[");
});

test("コアが保存を断ったら、前の値に戻して理由を出す", async () => {
  const core = await start();
  await open("display");
  core.rejectSave = { kind: "storage", detail: "disk full" };
  change($("#set-theme"), "dark");
  expect(prefs.theme).toBe("dark");
  await until(() => $(".err") !== null);
  expect(text(".err")).toBe("設定を保存できませんでした。前の値に戻しました。");
  expect(prefs.theme).toBe("system");
  expect(document.documentElement.hasAttribute("data-theme")).toBe(false);

  core.rejectSave = { kind: "invalid_argument", reason: "font_size" };
  change($("#set-font-size"), "19");
  await until(() => text(".err") === "この値は設定できません。前の値に戻しました。");
  expect(prefs.fontSize).toBe(15);
  // 次の保存を始めると消える
  change($("#set-layout"), "C");
  await until(() => $(".err") === null);
});

test("起動のとき、外枠を出す前に保存した設定を読み、その値で描く", async () => {
  const core = mockCore({ theme: "dark", locale: "en", fontSize: 17, layout: "C", noteFont: "mono", carryoverReviewThreshold: 5 });
  let open!: () => void;
  core.gate = new Promise<void>((r) => (open = r));
  app = mount(App, { target: document.body });
  await until(() => core.count("get_settings") === 1);
  // 読み終えるまでは、外枠も本文も出さない（既定の値で描いて、後から切り替えない）
  expect($("main.content")).toBeNull();
  expect($(".startup")).toBeNull();
  open();
  await until(() => $("main.content") !== null);
  // 最初に描いたときには、保存した値が入っている（等幅のノートは、読み込み終えてから描く）
  expect(prefs.noteFont).toBe("mono");
  expect(document.documentElement.dataset.theme).toBe("dark");
  expect(document.documentElement.lang).toBe("en");
  expect(document.documentElement.style.getPropertyValue("--fs-setting")).toBe("17");
  await until(() => $(".board")?.dataset.layout === "C");
  expect(prefs.reviewThreshold).toBe(5);
});

test("設定を読めなかったときは、既定の値で続ける", async () => {
  const core = mockCore();
  core.answer.get_settings = { kind: "storage", detail: "x" };
  app = mount(App, { target: document.body });
  await until(() => $("main.content") !== null);
  expect(prefs.theme).toBe("system");
  expect(prefs.layout).toBe("A");
});

test("サイドバーの開閉は、広いウィンドウの整理モードの開け閉めだけを保存し、起動のとき当てる", async () => {
  const core = await start();
  expect($(".side")).not.toBeNull();
  press({ key: "\\", code: "Backslash", ctrlKey: true });
  expect($(".side")).toBeNull();
  await until(() => core.count("save_settings") === 1);
  expect(core.last("save_settings")).toEqual({ patch: { sidebarOpen: false } });
  press({ key: "\\", code: "Backslash", ctrlKey: true });
  await until(() => core.count("save_settings") === 2);
  expect(core.last("save_settings")).toEqual({ patch: { sidebarOpen: true } });
  // ノートモードの間の開け閉めは保存しない
  press({ key: "j", code: "KeyJ", ctrlKey: true });
  press({ key: "\\", code: "Backslash", ctrlKey: true });
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(core.count("save_settings")).toBe(2);

  unmount(app!);
  app = undefined;
  document.body.innerHTML = "";
  await new Promise((resolve) => setTimeout(resolve, 0));
  clearMocks();
  prefs.apply({ ...DEFAULTS, sidebarOpen: true });
  mockCore({ sidebarOpen: false });
  app = mount(App, { target: document.body });
  await until(() => $("main.content")?.dataset.date === TODAY);
  expect($(".side")).toBeNull();
});

test("凪メーターの時間帯の設定で、太陽の位置が変わる", async () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 10, 3, 12, 0));
  await start({ meterStart: "08:00", meterEnd: "16:00" });
  // 12:00 は 8:00〜16:00 の真ん中（既定の 9:00〜19:00 なら 0.3）
  expect(($(".main") as HTMLElement).style.getPropertyValue("--sun-t")).toBe("0.5");
});

test("データ：保存先を変える。クラウド同期フォルダなら警告を添え、確かめてから移す", async () => {
  const core = await start();
  await open("data");
  await until(() => text('[data-pane="data"]').includes("/data"));
  core.answer.check_data_dir = "dropbox";
  picked.values = ["/home/u/Dropbox/yunagi"];
  button("保存先を変える").click();
  await until(() => $(".confirm") !== null);
  expect(picked.asked).toEqual(["移す先のフォルダを選ぶ"]);
  expect(core.last("check_data_dir")).toEqual({ dir: "/home/u/Dropbox/yunagi" });
  expect(text(".confirm")).toContain("/home/u/Dropbox/yunagi へ移します");
  expect(text(".confirm .warn")).toContain("Dropbox の同期フォルダ");
  // 「同時に1台だけで開く場合に限る」は、判定にかかわらずいつも出す
  expect(text('[data-pane="data"]')).toContain("同時に1台だけで開く場合に限ります");
  // 選んだだけでは移さない。やめれば何も起きない
  expect(core.count("change_data_dir")).toBe(0);
  button("やめる").click();
  flushSync();
  expect($(".confirm")).toBeNull();
  expect(core.count("change_data_dir")).toBe(0);

  picked.values = ["/mnt/drive/yunagi"];
  core.answer.check_data_dir = null;
  button("保存先を変える").click();
  await until(() => $(".confirm") !== null);
  expect($(".confirm .warn")).toBeNull();
  button("移す").click();
  await until(() => text(".result") === "保存先を変えました。");
  expect(core.last("change_data_dir")).toEqual({ dir: "/mnt/drive/yunagi" });
  // 中身は同じなので、読み込み直さない
  expect(picked.reloads).toBe(0);
});

test("データ：保存先を変えられなかったとき、理由を出す。消せなかった元のファイルも伝える", async () => {
  const core = await start();
  await open("data");
  core.answer.change_data_dir = { kind: "rejected", reason: "target_has_data" };
  picked.values = ["/mnt/x"];
  button("保存先を変える").click();
  await until(() => $(".confirm") !== null);
  button("移す").click();
  await until(() => text(".result").includes("すでにデータベースかバックアップがあります"));
  expect($(".result.warn")).not.toBeNull();

  core.answer.change_data_dir = { leftovers: ["/old/yunagi.db-wal"] };
  picked.values = ["/mnt/y"];
  button("保存先を変える").click();
  await until(() => $(".confirm") !== null);
  button("移す").click();
  await until(() => text(".result").includes("消せなかったものが 1 件"));

  core.answer.change_data_dir = { kind: "storage", detail: "x" };
  picked.values = ["/mnt/z"];
  button("保存先を変える").click();
  await until(() => $(".confirm") !== null);
  button("移す").click();
  await until(() => text(".result") === "保存先を変えられませんでした。データは元のままです。");
});

test("データ：既存のフォルダの選び直しは、入力を送ってからデータを切り替え、画面を読み込み直す", async () => {
  const core = await start();
  await open("data");
  picked.values = ["/mnt/other"];
  button("既存のフォルダを選び直す").click();
  await until(() => $(".confirm") !== null);
  expect(picked.asked).toEqual(["データベースのあるフォルダを選ぶ"]);
  expect(text(".confirm")).toContain("/mnt/other にあるデータに切り替えます");
  expect(core.count("use_data_dir")).toBe(0);
  button("切り替える").click();
  await until(() => picked.reloads === 1);
  expect(core.last("use_data_dir")).toEqual({ dir: "/mnt/other" });

  // 使えないフォルダは、理由を出して読み込み直さない
  core.answer.use_data_dir = { kind: "rejected", reason: "no_database" };
  picked.values = ["/mnt/empty"];
  button("既存のフォルダを選び直す").click();
  await until(() => $(".confirm") !== null);
  button("切り替える").click();
  await until(() => $(".result.warn") !== null);
  expect(text(".result")).toContain("データベース（yunagi.db）が見つかりません");
  expect(picked.reloads).toBe(1);
});

test("データ：バックアップの一覧（種類と日時）を出し、確かめてから復元して、画面を読み込み直す", async () => {
  const core = await start();
  await open("data");
  await until(() => $$(".backup").length === 2);
  const rows = $$(".backup").map((r) => r.textContent?.replace(/\s+/g, " ").trim());
  expect(rows[0]).toMatch(/^定期 11\.02 Mon \d{4}\.\d\d\.\d\d \d\d:\d\d 戻す$/);
  expect(rows[1]).toMatch(/^更新前（スキーマ 2 → 3） \d{4}\.\d\d\.\d\d \d\d:\d\d 戻す$/);
  expect(text('[data-pane="data"]')).toContain("世代数は変えられません");

  $$(".backup .btn")[1].click();
  await until(() => $(".confirm") !== null);
  expect(text(".confirm")).toContain("更新前（スキーマ 2 → 3）");
  expect(text(".confirm")).toContain("「復元前」のバックアップに残します");
  expect(core.count("restore_backup")).toBe(0);
  button("やめる").click();
  flushSync();
  expect(core.count("restore_backup")).toBe(0);

  $$(".backup .btn")[0].click();
  await until(() => $(".confirm") !== null);
  ($(".confirm .primary") as HTMLElement).click();
  await until(() => picked.reloads === 1);
  expect(core.last("restore_backup")).toEqual({ backup: BACKUPS[0].path });
});

test("データ：復元に失敗したら、データは元のままと伝え、読み込み直さない", async () => {
  const core = await start();
  await open("data");
  await until(() => $$(".backup").length === 2);
  core.answer.restore_backup = { kind: "rejected", reason: "corrupted" };
  $$(".backup .btn")[0].click();
  await until(() => $(".confirm") !== null);
  ($(".confirm .primary") as HTMLElement).click();
  await until(() => $(".result.warn") !== null);
  expect(text(".result")).toBe("壊れているため使えません。");
  expect(picked.reloads).toBe(0);
});

test("データ：Markdown 書き出し。期間と書き出し先を選んで書き出し、数とフォルダを短時間だけ出す", async () => {
  const core = await start();
  await open("data");
  const run = () => $$("[data-settings] button").find((b) => b.textContent?.trim() === "書き出す") as HTMLButtonElement;
  // フォルダを選ぶまでは書き出せない。期間の既定は今日
  await until(() => ($$('input[type="date"]')[0] as HTMLInputElement).value === TODAY);
  expect(run().disabled).toBe(true);
  change($$('input[type="date"]')[0], "2026-11-02");
  picked.values = ["/out"];
  button("フォルダを選ぶ").click();
  await until(() => !run().disabled);
  expect(picked.asked.at(-1)).toBe("書き出し先のフォルダを選ぶ");
  vi.useFakeTimers({ toFake: ["setTimeout"] });
  run().click();
  await vi.waitFor(() => expect(text(".result")).toBe("2 件のファイルを /out に書き出しました。"));
  expect(core.last("export_markdown")).toEqual({ from: "2026-11-02", to: TODAY, dir: "/out" });
  // 短時間だけ出す
  vi.advanceTimersByTime(8000);
  flushSync();
  expect(text(".result")).toBe("");
  vi.useRealTimers();

  // 書き出すものが無い期間は、ファイルを作らなかったと伝える
  core.answer.export_markdown = [];
  run().click();
  await until(() => text(".result") === "書き出すものがない期間です。ファイルは作っていません。");
});

test("データ：書き出しの失敗は、書き出し先の問題とデータを読めない問題を分けて伝える", async () => {
  const core = await start();
  await open("data");
  const run = () => $$("[data-settings] button").find((b) => b.textContent?.trim() === "書き出す") as HTMLButtonElement;
  picked.values = ["/out"];
  button("フォルダを選ぶ").click();
  await until(() => !run().disabled);

  core.answer.export_markdown = { kind: "export_failed", replaced: 0, detail: "x" };
  run().click();
  await until(() => text(".result").startsWith("書き出し先に書き込めませんでした"));
  expect(text(".result")).not.toContain("書き換え済み");

  core.answer.export_markdown = { kind: "export_failed", replaced: 2, detail: "x" };
  run().click();
  await until(() => text(".result").includes("2 件までは書き換え済みです。"));

  core.answer.export_markdown = { kind: "storage", detail: "x" };
  run().click();
  await until(() => text(".result") === "データを読めなかったため、書き出せませんでした。");
});

test("エージェント区分は、変更履歴を開く入口と、まだ変えられない設定の場所を置く", async () => {
  await start();
  await open("agents");
  expect($$('[data-pane="agents"] .placeholder')).toHaveLength(3);
  // 変更履歴（S-06）はまだ無いので、押しても設定は閉じない
  button("変更履歴を開く").click();
  flushSync();
  expect(dialog()).not.toBeNull();
});

test("区分の一覧は ↑/↓ で移り、Tab は設定の中だけを回る", async () => {
  await start();
  await open();
  await until(() => document.activeElement?.getAttribute("data-section") === "general");
  press({ key: "ArrowDown" }, document.activeElement!);
  await until(() => document.activeElement?.getAttribute("data-section") === "display");
  expect($('[data-pane="display"]')).not.toBeNull();
  press({ key: "ArrowUp" }, document.activeElement!);
  await until(() => document.activeElement?.getAttribute("data-section") === "general");
  press({ key: "ArrowUp" }, document.activeElement!);
  await until(() => document.activeElement?.getAttribute("data-section") === "agents");

  // 最後のボタンから Tab で先頭へ戻る
  const items = $$("[data-settings] button, [data-settings] input, [data-settings] select");
  items.at(-1)!.focus();
  const tab = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
  items.at(-1)!.dispatchEvent(tab);
  expect(tab.defaultPrevented).toBe(true);
  expect(dialog()!.contains(document.activeElement)).toBe(true);
});
