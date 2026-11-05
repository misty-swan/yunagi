import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { SearchHit } from "./bindings";
import { MESSAGES } from "./i18n";
import {
  hitView,
  matchCommands,
  Palette,
  paletteCommands,
  SEARCH_DELAY_MS,
  SEARCH_LIMIT,
  type CommandContext,
} from "./palette.svelte";

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

function hit(body: string, over: Partial<SearchHit> = {}): SearchHit {
  return { kind: "todo", refId: "01J", date: "2026-11-02", body, resolutionNote: null, carriedTo: null, ...over };
}

/** 答えを手で返す模擬の検索。 */
function deferredSearch() {
  const calls: { text: string; limit: number; resolve: (h: SearchHit[]) => void; reject: (e: unknown) => void }[] = [];
  const search = (text: string, limit: number) =>
    new Promise<SearchHit[]>((resolve, reject) => calls.push({ text, limit, resolve, reject }));
  return { calls, search };
}

test("1文字ずつ打っても、打ち止めてから1回だけ検索を送る（IME の変換中は送らない）", async () => {
  const { calls, search } = deferredSearch();
  const p = new Palette(search);
  p.show();
  for (const q of ["れ", "れー", "れーと", "レート"]) {
    p.input(q, q !== "レート");
    // 変換中は送らないので、探している途中にもしない
    expect(p.searching).toBe(q === "レート");
    await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS / 2);
  }
  expect(calls).toHaveLength(0);
  await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
  expect(calls.map((c) => [c.text, c.limit])).toEqual([["レート", SEARCH_LIMIT]]);
  expect(p.searching).toBe(true);
  calls[0].resolve([hit("レート制限")]);
  await vi.advanceTimersByTimeAsync(0);
  expect(p.searching).toBe(false);
  expect(p.viewsFor).toBe("レート");
  expect(p.views[0].body.mark).toEqual({ from: 0, to: 3 });
});

test("後から送った検索の答えだけを使い、答えが届くまでは前の答えを残す", async () => {
  const { calls, search } = deferredSearch();
  const p = new Palette(search);
  p.show();
  p.input("定例");
  await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
  calls[0].resolve([hit("定例の議事")]);
  await vi.advanceTimersByTimeAsync(0);
  p.input("定例会");
  await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
  p.input("定例会議");
  await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
  expect(calls.map((c) => c.text)).toEqual(["定例", "定例会", "定例会議"]);
  // 前の答えは、その検索語のまま残っている
  expect(p.viewsFor).toBe("定例");
  // 新しいほうが先に届き、古いほうは後から届いても捨てる
  calls[2].resolve([hit("定例会議の準備")]);
  await vi.advanceTimersByTimeAsync(0);
  calls[1].resolve([hit("古い答え")]);
  await vi.advanceTimersByTimeAsync(0);
  expect(p.viewsFor).toBe("定例会議");
  expect(p.views.map((v) => v.hit.body)).toEqual(["定例会議の準備"]);
});

test("空にしたら結果を消し、検索できなければ知らせる。閉じたら待っている答えを捨てる", async () => {
  const { calls, search } = deferredSearch();
  const p = new Palette(search);
  p.show();
  p.input("abc");
  await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
  calls[0].reject({ kind: "storage" });
  await vi.advanceTimersByTimeAsync(0);
  expect(p.failed).toBe(true);
  // ほかの検索語を打てば、前の失敗は出さない（探している途中にする）
  p.input("abx");
  expect(p.failed).toBe(false);
  expect(p.searching).toBe(true);
  p.input("  ");
  expect(p.failed).toBe(false);
  expect(p.views).toEqual([]);
  await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
  expect(calls).toHaveLength(1);

  p.input("abcd");
  await vi.advanceTimersByTimeAsync(SEARCH_DELAY_MS);
  p.close();
  calls[1].resolve([hit("abcd")]);
  await vi.advanceTimersByTimeAsync(0);
  expect(p.views).toEqual([]);
  // 開き直すと空から
  p.show();
  expect(p.query).toBe("");
});

test("解消メモで一致した障害は、解消メモの一部も出す", () => {
  const v = hitView(hit("API が遅い", { kind: "blocker", resolutionNote: "キャッシュを足して解消" }), "キャッシュ");
  expect(v.body.mark).toBeNull();
  expect(v.note?.text).toBe("キャッシュを足して解消");
  expect(v.note?.mark).toEqual({ from: 0, to: 5 });
  expect(hitView(hit("API が遅い", { resolutionNote: "API を直した" }), "API").note).toBeNull();
});

const ctx: CommandContext = {
  mode: "board",
  sidebarOpen: true,
  layout: "B",
  carryPending: false,
  saveFailing: false,
  updateAvailable: false,
  mac: false,
};

test("コマンドの一覧：持ち越し・保存の失敗はその間だけ出し、使用中とキーを添える", () => {
  const ja = MESSAGES.ja;
  const en = MESSAGES.en;
  const list = paletteCommands(ctx, ja, en);
  expect(list.map((c) => c.id)).toEqual([
    "today",
    "mode",
    "archive",
    "agents",
    "settings",
    "sidebar",
    "layoutA",
    "layoutB",
    "layoutC",
  ]);
  expect(list.find((c) => c.id === "layoutB")?.active).toBe(true);
  expect(list.find((c) => c.id === "layoutA")?.active).toBe(false);
  expect(list.find((c) => c.id === "sidebar")).toMatchObject({ label: "サイドバーを閉じる", key: "Ctrl+\\" });
  expect(list.find((c) => c.id === "archive")?.key).toBe("Ctrl+Shift+L");
  expect(list.find((c) => c.id === "agents")?.key).toBeNull();

  const more = paletteCommands({ ...ctx, carryPending: true, saveFailing: true, mode: "note", mac: true }, ja, en);
  expect(more.slice(0, 2).map((c) => c.id)).toEqual(["carryover", "saveDetail"]);
  expect(more.find((c) => c.id === "mode")).toMatchObject({ label: "整理モードにする", key: "⌘J" });
});

test("「更新して再起動」は、新しい版があるときだけ出す（保存の失敗の次）", () => {
  const ja = MESSAGES.ja;
  const en = MESSAGES.en;
  expect(paletteCommands(ctx, ja, en).some((c) => c.id === "update")).toBe(false);
  const list = paletteCommands({ ...ctx, saveFailing: true, updateAvailable: true }, ja, en);
  expect(list.slice(0, 3).map((c) => c.id)).toEqual(["saveDetail", "update", "today"]);
  expect(list.find((c) => c.id === "update")).toMatchObject({ label: "更新して再起動", key: null });
  expect(matchCommands(list, "update and restart").map((c) => c.id)).toEqual(["update"]);
});

test("コマンドは、どちらの言語の名前の一部でも探せる（大文字・小文字・空白は問わない）", () => {
  const list = paletteCommands({ ...ctx, carryPending: true }, MESSAGES.ja, MESSAGES.en);
  expect(matchCommands(list, "持ち越し").map((c) => c.id)).toEqual(["carryover"]);
  expect(matchCommands(list, "carry over").map((c) => c.id)).toEqual(["carryover"]);
  expect(matchCommands(list, "ARCHIVE").map((c) => c.id)).toEqual(["archive"]);
  expect(matchCommands(list, "レイアウト").map((c) => c.id)).toEqual(["layoutA", "layoutB", "layoutC"]);
  expect(matchCommands(list, "")).toHaveLength(list.length);
});
