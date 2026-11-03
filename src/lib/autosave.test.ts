import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { Item, TextSave } from "./bindings";
import {
  DocSaver,
  ItemSaves,
  RETRY_DELAYS_MS,
  SAVE_DELAY_MS,
  SaveStatus,
  type DocEditor,
} from "./autosave.svelte";

beforeEach(() => {
  vi.useFakeTimers({ now: 1_800_000_000_000 });
});

afterEach(() => {
  vi.useRealTimers();
  clearMocks();
});

const DATE = "2026-11-03";
const ID = "01JC0000000000000000000000";

/** 模擬のエディタ。 */
function editor(text = ""): DocEditor & { value: string } {
  return {
    value: text,
    text() {
      return this.value;
    },
    replace(t: string) {
      this.value = t;
    },
  };
}

interface Call {
  cmd: string;
  args: Record<string, unknown>;
  at: number;
}

/** `handle` を模擬のコアにする。呼び出しを時刻つきで記録する。 */
function mockCore(handle: (cmd: string, args: Record<string, unknown>) => unknown | Promise<unknown>) {
  const calls: Call[] = [];
  mockIPC(async (cmd, args) => {
    const a = args as Record<string, unknown>;
    calls.push({ cmd, args: a, at: Date.now() });
    return handle(cmd, a);
  });
  return calls;
}

function item(text: string): Item {
  return {
    id: ID,
    date: DATE,
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
  };
}

test("入力が止まってから SAVE_DELAY_MS 後に保存し、同じ本文なら送らない", async () => {
  let rev = 3;
  const calls = mockCore((): TextSave => ({ result: "saved", rev: ++rev, updated: true }));
  const status = new SaveStatus();
  const ed = editor("a");
  const saver = new DocSaver(status, "note", DATE, { body: "a", rev: 3 }, ed);

  saver.edited();
  expect(status.saving).toBe(false);
  ed.value = "ab";
  saver.edited();
  expect(status.saving).toBe(true);
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS - 1);
  expect(calls).toHaveLength(0);
  await vi.advanceTimersByTimeAsync(1);
  expect(calls.map((c) => c.args)).toEqual([{ doc: "note", date: DATE, body: "ab", baseRev: 3 }]);
  await vi.advanceTimersByTimeAsync(0);
  expect(status.saving).toBe(false);
  expect(saver.saved).toEqual({ body: "ab", rev: 4 });
  // 次の保存は、成功の応答の版をもとにする
  ed.value = "abc";
  saver.edited();
  await saver.flush();
  expect(calls[1].args.baseRev).toBe(4);
  expect(calls[1].at - calls[0].at).toBeLessThan(SAVE_DELAY_MS);
});

test("入力が続いても待ち直さず、どの入力も SAVE_DELAY_MS 以内に送る（強制終了で失うのは直前の分だけ）", async () => {
  let rev = 0;
  const calls = mockCore((): TextSave => ({ result: "saved", rev: ++rev, updated: true }));
  const ed = editor("");
  const saver = new DocSaver(new SaveStatus(), "note", DATE, { body: "", rev: 0 }, ed);
  const typed: { at: number; length: number }[] = [];
  // 2 秒間、40ms ごとに1文字ずつ打つ
  for (let i = 0; i < 50; i++) {
    ed.value += "x";
    saver.edited();
    typed.push({ at: Date.now(), length: ed.value.length });
    await vi.advanceTimersByTimeAsync(40);
  }
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  for (const t of typed) {
    const sent = calls.find((c) => (c.args.body as string).length >= t.length);
    expect(sent, `typed at ${t.at}`).toBeDefined();
    expect(sent!.at - t.at).toBeLessThanOrEqual(SAVE_DELAY_MS);
  }
  expect(saver.saved.body).toBe(ed.value);
});

test("保存は一度に1つだけ送り、送っている間の入力は応答の後に送る", async () => {
  let release: (() => void) | undefined;
  let rev = 1;
  const calls = mockCore(async () => {
    await new Promise<void>((r) => (release = r));
    return { result: "saved", rev: ++rev, updated: true } satisfies TextSave;
  });
  const ed = editor("a");
  const saver = new DocSaver(new SaveStatus(), "note", DATE, { body: "a", rev: 1 }, ed);
  ed.value = "ab";
  saver.edited();
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(calls).toHaveLength(1);
  ed.value = "abc";
  saver.edited();
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 3);
  expect(calls).toHaveLength(1);
  release!();
  // 入力から SAVE_DELAY_MS を過ぎているので、応答の後すぐに送る
  await vi.advanceTimersByTimeAsync(0);
  expect(calls).toHaveLength(2);
  expect(calls[1].args).toMatchObject({ body: "abc", baseRev: 2 });
  release!();
  await vi.advanceTimersByTimeAsync(0);
  expect(saver.saved).toEqual({ body: "abc", rev: 3 });
});

test("版が古ければ、最新を取り込んでから、利用者の入力を残したまま保存し直す", async () => {
  const calls = mockCore((_cmd, args): TextSave =>
    args.baseRev === 1
      ? { result: "stale", body: "a\nb\nエージェント", rev: 2 }
      : { result: "saved", rev: 3, updated: true },
  );
  const ed = editor("a\nb");
  const saver = new DocSaver(new SaveStatus(), "note", DATE, { body: "a\nb", rev: 1 }, ed);
  ed.value = "a\nB";
  saver.edited();
  await saver.flush();
  expect(calls.map((c) => c.args)).toEqual([
    { doc: "note", date: DATE, body: "a\nB", baseRev: 1 },
    { doc: "note", date: DATE, body: "a\nB\nエージェント", baseRev: 2 },
  ]);
  expect(ed.value).toBe("a\nB\nエージェント");
  expect(saver.saved).toEqual({ body: "a\nB\nエージェント", rev: 3 });
});

test("エージェントの変更の取り込みは、送っている間は応答の後に行う", async () => {
  let release: (() => void) | undefined;
  mockCore(async () => {
    await new Promise<void>((r) => (release = r));
    return { result: "saved", rev: 2, updated: true } satisfies TextSave;
  });
  const ed = editor("a");
  const saver = new DocSaver(new SaveStatus(), "note", DATE, { body: "a", rev: 1 }, ed);
  ed.value = "a\nmine";
  saver.edited();
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  // 自分の保存（rev 2）の後のエージェントの追記（rev 3）を、応答より先に読み直した
  saver.absorb({ body: "a\nmine\nagent", rev: 3 });
  expect(ed.value).toBe("a\nmine");
  release!();
  await vi.advanceTimersByTimeAsync(0);
  expect(ed.value).toBe("a\nmine\nagent");
  expect(saver.saved).toEqual({ body: "a\nmine\nagent", rev: 3 });
  expect(saver.unsavedLines()).toEqual([]);
  // 古い版の知らせは無視する
  saver.absorb({ body: "old", rev: 2 });
  expect(ed.value).toBe("a\nmine\nagent");
});

test("保存に失敗したら、内容を持ったまま再試行し、成功するまで「保存できていません」", async () => {
  let fail = true;
  let rev = 1;
  const calls = mockCore(() => {
    if (fail) throw { kind: "storage", detail: "disk is full" };
    return { result: "saved", rev: ++rev, updated: true } satisfies TextSave;
  });
  const status = new SaveStatus();
  const ed = editor("a");
  const saver = new DocSaver(status, "note", DATE, { body: "a", rev: 1 }, ed);
  ed.value = "ab";
  saver.edited();
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(status.failing).toBe(true);
  expect(status.error).toEqual({ kind: "storage", detail: "disk is full" });
  expect(status.saving).toBe(true);
  // 入力は続けられる。再試行は間を空けて、そのときの内容を送る
  ed.value = "abc";
  saver.edited();
  await vi.advanceTimersByTimeAsync(RETRY_DELAYS_MS[0] - 1);
  expect(calls).toHaveLength(1);
  await vi.advanceTimersByTimeAsync(1);
  expect(calls).toHaveLength(2);
  expect(calls[1].args.body).toBe("abc");
  // 2回目の失敗の後は、次の間を空ける
  await vi.advanceTimersByTimeAsync(RETRY_DELAYS_MS[1] - 1);
  expect(calls).toHaveLength(2);
  // 「再試行」の操作で、待たずに送る
  fail = false;
  status.retryNow();
  await vi.advanceTimersByTimeAsync(0);
  expect(calls).toHaveLength(3);
  expect(status.failing).toBe(false);
  expect(status.error).toBeNull();
  expect(status.saving).toBe(false);
  expect(saver.saved).toEqual({ body: "abc", rev: 2 });
});

test("項目は、待つ間に変えた欄を1つの保存にまとめ、状態は最初の「変える前の値」と時刻を残す", async () => {
  let release: (() => void) | undefined;
  const calls = mockCore(async (cmd) => {
    expect(cmd).toBe("save_item");
    await new Promise<void>((r) => (release = r));
    return { updated: true, item: item("x") };
  });
  const status = new SaveStatus();
  const saved: string[] = [];
  const items = new ItemSaves(status, { saved: (key) => saved.push(key) });
  items.editText(ID, "a");
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(calls).toHaveLength(1);
  expect(items.unsavedFields()).toEqual([{ itemId: ID, field: "text" }]);

  const first = Date.now();
  items.setClosed(ID, "open", first);
  await vi.advanceTimersByTimeAsync(10);
  items.editText(ID, "ab");
  items.setClosed(ID, "done", null);
  await vi.advanceTimersByTimeAsync(10);
  const last = Date.now();
  items.setClosed(ID, "open", last);
  items.editText(ID, "abc");
  release!();
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(calls).toHaveLength(2);
  expect(calls[1].args).toEqual({
    id: ID,
    fields: { text: "abc", status: { before: "open", closedAt: last, changedAt: first } },
  });
  release!();
  await vi.advanceTimersByTimeAsync(0);
  expect(items.unsavedFields()).toEqual([]);
  expect(status.saving).toBe(false);
  expect(saved).toEqual([ID, ID]);
});

test("まだ作っていない項目は、作り終えてから残りの欄を送る", async () => {
  const calls = mockCore((cmd) => {
    if (cmd === "add_item") return item("新しい");
    return { updated: true, item: item("新しい") };
  });
  const items = new ItemSaves(new SaveStatus());
  items.create("tmp-1", { date: DATE, kind: "todo", position: { at: "last" } }, "新");
  items.editText("tmp-1", "新しい");
  items.setClosed("tmp-1", "open", 1);
  expect(items.idOf("tmp-1")).toBeNull();
  await items.flush("tmp-1");
  expect(calls.map((c) => [c.cmd, c.args])).toEqual([
    ["add_item", { date: DATE, kind: "todo", text: "新しい", position: { at: "last" } }],
    ["save_item", { id: ID, fields: { status: { before: "open", closedAt: 1, changedAt: expect.any(Number) } } }],
  ]);
  expect(items.idOf("tmp-1")).toBe(ID);
  // 削除などは、保存を待ってから
  const deleted = await items.run("tmp-1", async (id) => id);
  expect(deleted).toBe(ID);
});

test("送り直しても成功しない失敗は再試行せず、知らせる", async () => {
  const calls = mockCore(() => {
    throw { kind: "conflict", reason: "carried_over", carriedToId: ID, carriedToDate: DATE };
  });
  const status = new SaveStatus();
  const rejected: unknown[] = [];
  const items = new ItemSaves(status, { rejected: (key, e, patch) => rejected.push([key, e.kind, patch]) });
  items.editText(ID, "a");
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS + RETRY_DELAYS_MS[0] * 2);
  expect(calls).toHaveLength(1);
  expect(rejected).toEqual([[ID, "conflict", { text: "a" }]]);
  expect(status.failing).toBe(false);
  expect(status.saving).toBe(false);
});
