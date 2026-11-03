import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { EVENTS, type CarryoverList, type CarryoverStatus, type CommandError, type Item } from "../lib/bindings";
import { Carryover } from "../lib/carryover.svelte";
import { prefs } from "../lib/prefs.svelte";
import CarryoverPanel from "./CarryoverPanel.svelte";

const FROM = "2026-10-28";
const A = "01JC00000000000000000000A0";
const B = "01JC00000000000000000000B0";
const C = "01JC00000000000000000000C0";
const D = "01JC00000000000000000000D0";

let app: ReturnType<typeof mount> | undefined;
let carry: Carryover | undefined;

beforeEach(() => {
  prefs.lang = "ja";
});

afterEach(async () => {
  if (app) unmount(app);
  app = undefined;
  await carry?.stop();
  carry = undefined;
  document.body.innerHTML = "";
  await new Promise((r) => setTimeout(r, 0));
  clearMocks();
});

function item(id: string, text: string, over: Partial<Item> = {}): Item {
  return {
    id,
    date: FROM,
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

/** 模擬のコアの持ち越し（テストの中で変えると、次の問い合わせから変わる）。 */
interface Fake {
  status: CarryoverStatus | null;
  list: CarryoverList | null;
  reshown: boolean;
  confirmError: CommandError | null;
  /** 一覧の読み込みを、この回数だけ失敗させる。 */
  listFailures?: number;
}

function mockCore(fake: Fake) {
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  mockIPC(
    (cmd, args) => {
      const a = args as Record<string, unknown>;
      calls.push({ cmd, args: a });
      switch (cmd) {
        case "carryover_status":
          return fake.status;
        case "carryover_candidates":
          if ((fake.listFailures ?? 0) > 0) {
            fake.listFailures! -= 1;
            throw { kind: "storage", detail: "disk" };
          }
          return fake.list;
        case "mark_carryover_reshown": {
          const r = fake.reshown;
          fake.reshown = false;
          return r;
        }
        case "postpone_carryover":
          return null;
        case "confirm_carryover":
          if (fake.confirmError) throw fake.confirmError;
          fake.status = null;
          return { created: [], closed: [] };
      }
      throw new Error(`unexpected command ${cmd}`);
    },
    { shouldMockEvents: true },
  );
  return calls;
}

function defaultFake(over: Partial<Fake> = {}): Fake {
  return {
    status: { from: FROM, prompt: "show" },
    list: {
      from: FROM,
      reviewThreshold: 3,
      items: [
        { item: item(A, "3回持ち越した", { carryCount: 3 }), review: true },
        { item: item(B, "ふつう", { carryCount: 1 }), review: false },
        { item: item(C, "権限待ち", { kind: "blocker", carryCount: 3 }), review: false },
      ],
    },
    reshown: true,
    confirmError: null,
    ...over,
  };
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
const check = (id: string) => $(`input[data-carry-check="${id}"]`) as HTMLInputElement;

function press(target: Element, key: string, init: KeyboardEventInit = {}) {
  target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }));
  flushSync();
}

async function start(fake = defaultFake()) {
  const calls = mockCore(fake);
  const onOpen = vi.fn();
  carry = new Carryover(onOpen);
  app = mount(CarryoverPanel, { target: document.body, props: { carry } });
  // イベントを聞き始めるのを待つ
  await new Promise((r) => setTimeout(r, 0));
  return { calls, onOpen, fake, carry };
}

function confirmed(calls: { cmd: string; args: Record<string, unknown> }[]) {
  return calls.filter((c) => c.cmd === "confirm_carryover").map((c) => c.args.decision);
}

test("初回表示で開き、見出し・TODO と障害・回数・見直しの選択肢を出す。開いたときは全部にチェック", async () => {
  const { carry, onOpen } = await start();
  await carry.started();
  await until(() => $("[data-carryover]") !== null && $$("li[data-carry-row]").length === 3);
  expect(onOpen).toHaveBeenCalledTimes(1);
  expect(text($(".ph .label"))).toBe("CARRY OVER");
  expect(text($("#carry-title"))).toBe("前回の日（10.28 Wed）に残った項目");
  expect($$("h3").map(text)).toEqual(["TODO", "BLOCKERS"]);
  expect($$("li[data-carry-row]").map((li) => li.dataset.carryRow)).toEqual([A, B, C]);
  expect([A, B, C].map((id) => check(id).checked)).toEqual([true, true, true]);
  // 見直しの選択肢は、回数に達した TODO にだけ（障害には回数だけ）
  expect($$("[data-carry-review]").map((b) => b.closest("li")?.dataset.carryRow)).toEqual([A, A]);
  expect(text($(`li[data-carry-row="${A}"] .review .msg`))).toBe("3回持ち越しています。分割しますか？");
  expect($$("span.carry").map((e) => [e.getAttribute("aria-label"), e.classList.contains("warn")])).toEqual([
    ["持ち越し 3 回", true],
    ["持ち越し 1 回", false],
    ["持ち越し 3 回", true],
  ]);
  // 先頭の行にフォーカス
  await until(() => document.activeElement === check(A));
});

test("j/k・↑/↓ で移り、Space でチェックを外し、Enter で選んだものを持ち越して閉じる", async () => {
  const { carry, calls } = await start();
  await carry.started();
  await until(() => document.activeElement === check(A));
  press(check(A), "j");
  await until(() => document.activeElement === check(B));
  press(check(B), "ArrowDown");
  await until(() => document.activeElement === check(C));
  press(check(C), "j");
  await until(() => document.activeElement === check(C));
  press(check(C), "k");
  await until(() => document.activeElement === check(B));
  // Space はチェック欄の既定の動き（jsdom では click で代える）
  check(B).click();
  flushSync();
  expect(check(B).checked).toBe(false);
  expect($(`li[data-carry-row="${B}"]`)?.classList.contains("off")).toBe(true);
  press(check(B), "Enter");
  await until(() => $("[data-carryover]") === null);
  expect(confirmed(calls)).toEqual([{ from: FROM, selection: { select: "selected", ids: [A, C] }, reviews: [] }]);
  expect(carry.pending).toBeNull();
});

test("見直しの「分割する」で子を入力でき、どのボタンでも反映する。「やめる」はもう一度押すと外す", async () => {
  const { carry, calls } = await start();
  await carry.started();
  await until(() => $$("li[data-carry-row]").length === 3);
  const split = $(`li[data-carry-row="${A}"] [data-carry-review="split"]`)!;
  split.click();
  await until(() => document.activeElement === $(`textarea[data-carry-split="${A}"]`));
  expect(split.getAttribute("aria-pressed")).toBe("true");
  expect(check(A).checked).toBe(false);
  expect(text($(`li[data-carry-row="${A}"] .cstate`))).toBe("分割する");
  const ta = $(`textarea[data-carry-split="${A}"]`) as HTMLTextAreaElement;
  ta.value = "調べる\n直す";
  ta.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
  // 入力欄の Enter は改行で、確定しない。Esc で行のチェック欄へ戻る
  press(ta, "Enter");
  expect(confirmed(calls)).toEqual([]);
  press(ta, "Escape");
  await until(() => document.activeElement === check(A));
  expect($("[data-carryover]")).not.toBeNull();

  // 「やめる」に切り替えて、もう一度押すと外れてチェックが戻る
  const drop = $(`li[data-carry-row="${A}"] [data-carry-review="drop"]`)!;
  drop.click();
  flushSync();
  expect($(`textarea[data-carry-split="${A}"]`)).toBeNull();
  drop.click();
  flushSync();
  expect(drop.getAttribute("aria-pressed")).toBe("false");
  expect(check(A).checked).toBe(true);

  split.click();
  await until(() => $(`textarea[data-carry-split="${A}"]`) !== null);
  const ta2 = $(`textarea[data-carry-split="${A}"]`) as HTMLTextAreaElement;
  ta2.value = "調べる\n直す";
  ta2.dispatchEvent(new Event("input", { bubbles: true }));
  flushSync();
  $("[data-act='none']")!.click();
  await until(() => $("[data-carryover]") === null);
  expect(confirmed(calls)).toEqual([
    {
      from: FROM,
      selection: { select: "none" },
      reviews: [{ id: A, review: { review: "split", children: ["調べる", "直す"] } }],
    },
  ]);
});

test("「すべて持ち越す」", async () => {
  const { carry, calls } = await start();
  await carry.started();
  await until(() => $$("li[data-carry-row]").length === 3);
  check(B).click();
  $("[data-act='all']")!.click();
  await until(() => $("[data-carryover]") === null);
  expect(confirmed(calls)).toEqual([{ from: FROM, selection: { select: "all" }, reviews: [] }]);
});

test("Esc で後回しにして閉じる。次に前面に出したとき1回だけ再表示し、以後はパレットから開く", async () => {
  const { carry, calls, fake } = await start();
  await carry.started();
  await until(() => document.activeElement === check(A));
  press(check(A), "Escape");
  await until(() => $("[data-carryover]") === null);
  expect(calls.some((c) => c.cmd === "postpone_carryover")).toBe(true);
  expect(confirmed(calls)).toEqual([]);

  fake.status = { from: FROM, prompt: "reshow_on_focus" };
  await carry.focused();
  await until(() => $("[data-carryover]") !== null);
  expect(calls.filter((c) => c.cmd === "mark_carryover_reshown")).toHaveLength(1);
  $("[data-act='later']")!.click();
  await until(() => $("[data-carryover]") === null);

  // 再表示を記録できなかった（記録済み）ときは出さない
  await carry.focused();
  flushSync();
  expect($("[data-carryover]")).toBeNull();
  fake.status = { from: FROM, prompt: "on_request" };
  await carry.focused();
  flushSync();
  expect($("[data-carryover]")).toBeNull();
  expect(carry.pending).toEqual({ from: FROM, prompt: "on_request" });
  await carry.openOnRequest();
  await until(() => $$("li[data-carry-row]").length === 3);
});

test("持ち越しが残っていなければ出さず、パレットからも開かない", async () => {
  const { carry } = await start(defaultFake({ status: null }));
  await carry.started();
  await carry.openOnRequest();
  flushSync();
  expect($("[data-carryover]")).toBeNull();
  expect(carry.pending).toBeNull();
});

test("出している間にデータが変わったら読み直す：外れた項目は選択ごと捨て、加わった項目はチェック無しで加える", async () => {
  const { carry, fake, calls } = await start();
  await carry.started();
  await until(() => $$("li[data-carry-row]").length === 3);
  $(`li[data-carry-row="${A}"] [data-carry-review="split"]`)!.click();
  check(B).click();
  flushSync();
  // エージェントが A を完了にし、B の本文を変え、前回の日に D を足した
  fake.list = {
    from: FROM,
    reviewThreshold: 3,
    items: [
      { item: item(B, "ふつう（変えた）", { carryCount: 1 }), review: false },
      { item: item(C, "権限待ち", { kind: "blocker", carryCount: 3 }), review: false },
      { item: item(D, "足した"), review: false },
    ],
  };
  await emit(EVENTS.changed, { by: "agent" });
  await until(() => $$("li[data-carry-row]").map((li) => li.dataset.carryRow).join() === [B, D, C].join());
  expect(text($(`li[data-carry-row="${B}"] .ctext`))).toBe("ふつう（変えた）");
  expect([B, C, D].map((id) => check(id).checked)).toEqual([false, true, false]);
  expect($(`textarea[data-carry-split="${A}"]`)).toBeNull();
  $("[data-act='selected']")!.click();
  await until(() => $("[data-carryover]") === null);
  expect(confirmed(calls)).toEqual([{ from: FROM, selection: { select: "selected", ids: [C] }, reviews: [] }]);
});

test("前回の日が変わっていたら確定せずに読み直し、知らせる。確定に失敗したら閉じずに知らせる", async () => {
  const fake = defaultFake({ confirmError: { kind: "conflict", reason: "previous_day_changed", previousDay: "2026-11-02" } });
  const { carry } = await start(fake);
  await carry.started();
  await until(() => $$("li[data-carry-row]").length === 3);
  fake.list = { from: "2026-11-02", reviewThreshold: 3, items: [{ item: item(D, "新しい日", { date: "2026-11-02" }), review: false }] };
  check(A).click();
  $("[data-act='selected']")!.click();
  await until(() => text($("#carry-title")) === "前回の日（11.02 Mon）に残った項目");
  expect(text($(".message"))).toBe("前回の日が変わったため、一覧を読み直しました。");
  // 開き直したものとして、全部にチェック
  expect(check(D).checked).toBe(true);

  fake.confirmError = { kind: "storage", detail: "disk full" };
  $("[data-act='all']")!.click();
  await until(() => text($(".message")) === "持ち越せませんでした。もう一度選べます。");
  expect($("[data-carryover]")).not.toBeNull();
  fake.confirmError = null;
  $("[data-act='all']")!.click();
  await until(() => $("[data-carryover]") === null);
});

test("ほかで持ち越しを終えていたら閉じる", async () => {
  const { carry } = await start(defaultFake({ confirmError: { kind: "conflict", reason: "carryover_done" } }));
  await carry.started();
  await until(() => $$("li[data-carry-row]").length === 3);
  $("[data-act='all']")!.click();
  await until(() => $("[data-carryover]") === null);
});

test("出している間は、ほかの画面のキー（window で受けるもの）を渡さない", async () => {
  const { carry } = await start();
  const seen = vi.fn();
  window.addEventListener("keydown", seen);
  try {
    await carry.started();
    await until(() => document.activeElement === check(A));
    press(check(A), "k", { ctrlKey: true });
    press(check(A), "z", { ctrlKey: true });
    // 背後にフォーカスがあっても渡さず、行へ戻す
    const outside = document.createElement("button");
    document.body.append(outside);
    outside.focus();
    press(outside, "k", { ctrlKey: true });
    expect(seen).not.toHaveBeenCalled();
    await until(() => document.activeElement === check(A));
  } finally {
    window.removeEventListener("keydown", seen);
  }
});

test("英語の文言", async () => {
  prefs.lang = "en";
  const { carry } = await start();
  await carry.started();
  await until(() => $$("li[data-carry-row]").length === 3);
  expect(text($("#carry-title"))).toBe("Left on 10.28 Wed");
  expect($$(".actions .btn").map((b) => b.firstChild?.textContent)).toEqual([
    "Carry over selected",
    "Carry over all",
    "Don’t carry over",
    "Later",
  ]);
});

test("一覧を読めなければ知らせて読み直し、読み終えるまでは確定できない", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    const { carry, calls } = await start(defaultFake({ listFailures: 1 }));
    void carry.started();
    await until(() => text($(".message")) === "一覧を読めませんでした。読み直しています。");
    expect(text($("#carry-title"))).toBe("読み込んでいます");
    expect(($("[data-act='selected']") as HTMLButtonElement).disabled).toBe(true);
    expect(($("[data-act='later']") as HTMLButtonElement).disabled).toBe(false);
    await carry.confirm("all");
    expect(confirmed(calls)).toEqual([]);
    expect($("[data-carryover]")).not.toBeNull();
    await vi.advanceTimersByTimeAsync(1000);
    await until(() => $$("li[data-carry-row]").length === 3);
    expect($(".message")).toBeNull();
    expect(($("[data-act='selected']") as HTMLButtonElement).disabled).toBe(false);
  } finally {
    vi.useRealTimers();
  }
});

test("前回の日が無くなったら空にし、確定のボタンで閉じる", async () => {
  const { carry, fake, calls } = await start();
  await carry.started();
  await until(() => $$("li[data-carry-row]").length === 3);
  fake.list = null;
  fake.status = null;
  await emit(EVENTS.changed, { by: "agent" });
  await until(() => $$("li[data-carry-row]").length === 0 && text($(".empty")) === "持ち越す項目はありません。");
  expect(text($("#carry-title"))).toBe("前回の日に残った項目");
  $("[data-act='selected']")!.click();
  await until(() => $("[data-carryover]") === null);
  expect(confirmed(calls)).toEqual([]);
  // 持ち越しの状態を読み直す
  await until(() => carry.pending === null);
});

test("閉じると、開く直前にフォーカスのあった所へ戻す", async () => {
  const { carry } = await start();
  const before = document.createElement("button");
  document.body.append(before);
  before.focus();
  await carry.started();
  await until(() => document.activeElement === check(A));
  press(check(A), "Escape");
  await until(() => $("[data-carryover]") === null && document.activeElement === before);
});

test("選んでいた行が読み直しで消えたら、同じ位置の行へ選択とフォーカスを移す", async () => {
  const { carry, fake } = await start();
  await carry.started();
  await until(() => document.activeElement === check(A));
  press(check(A), "j");
  await until(() => document.activeElement === check(B));
  fake.list = { ...fake.list!, items: fake.list!.items.filter((c) => c.item.id !== B) };
  await emit(EVENTS.changed, { by: "agent" });
  await until(() => document.activeElement === check(C));
  press(check(C), "k");
  await until(() => document.activeElement === check(A));
});

test("背後にフォーカスがあっても、Esc で後回し、Enter で選んだものを持ち越す", async () => {
  const { carry, calls } = await start();
  const outside = document.createElement("button");
  document.body.append(outside);
  await carry.started();
  await until(() => $$("li[data-carry-row]").length === 3);
  outside.focus();
  press(outside, "Escape");
  await until(() => $("[data-carryover]") === null);
  expect(calls.some((c) => c.cmd === "postpone_carryover")).toBe(true);

  await carry.openOnRequest();
  await until(() => $$("li[data-carry-row]").length === 3);
  outside.focus();
  press(outside, "Enter");
  await until(() => $("[data-carryover]") === null);
  expect(confirmed(calls)).toHaveLength(1);
});

test("前面に出したのとパレットから開くのが重なっても、1回だけ開き、再表示の記録も1回", async () => {
  const { carry, calls } = await start(defaultFake({ status: { from: FROM, prompt: "reshow_on_focus" } }));
  await Promise.all([carry.focused(), carry.openOnRequest(), carry.focused()]);
  await until(() => $$("li[data-carry-row]").length === 3);
  expect(calls.filter((c) => c.cmd === "mark_carryover_reshown")).toHaveLength(1);
  expect(calls.filter((c) => c.cmd === "carryover_candidates")).toHaveLength(1);
});

test("見直しの選択をコアが受け付けなかったら、閉じずに知らせる", async () => {
  const { carry } = await start(defaultFake({ confirmError: { kind: "invalid_argument", reason: "multiline_text" } }));
  await carry.started();
  await until(() => $$("li[data-carry-row]").length === 3);
  $("[data-act='all']")!.click();
  await until(() => text($(".message")) === "見直しの選択を受け付けられませんでした。選び直せます。");
  expect($("[data-carryover]")).not.toBeNull();
});
