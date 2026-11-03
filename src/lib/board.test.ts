import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { ItemSaves, RETRY_DELAYS_MS, SAVE_DELAY_MS, SaveStatus } from "./autosave.svelte";
import type { Board, BoardItem, Item } from "./bindings";
import { BoardModel, EDITING_MS, lockOf, showsMemo, type BoardNotice } from "./board.svelte";

beforeEach(() => {
  vi.useFakeTimers({ now: 1_800_000_000_000 });
});

afterEach(() => {
  vi.useRealTimers();
  clearMocks();
});

const DATE = "2026-11-03";
const A = "01JC00000000000000000000A0";
const B = "01JC00000000000000000000B0";
const C = "01JC00000000000000000000C0";
const NEW = "01JC00000000000000000000N0";

function item(id: string, text: string, over: Partial<Item> = {}): Item {
  return {
    id,
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
    ...over,
  };
}

function bi(i: Item, over: Partial<BoardItem> = {}): BoardItem {
  return { item: i, carriedTo: null, agentName: null, unseen: false, noteSource: null, ...over };
}

function board(todos: BoardItem[], blockers: BoardItem[] = [], date = DATE): Board {
  return { date, todos, blockers, reflection: "", note: { headings: [], length: 0 }, reflectionRev: 0, noteRev: 0 };
}

interface Call {
  cmd: string;
  args: Record<string, unknown>;
}

function setup(handle: (cmd: string, args: Record<string, unknown>) => unknown = () => null) {
  const calls: Call[] = [];
  mockIPC(async (cmd, args) => {
    const a = args as Record<string, unknown>;
    calls.push({ cmd, args: a });
    return handle(cmd, a);
  });
  const notices: BoardNotice[] = [];
  const reload = vi.fn();
  const status = new SaveStatus();
  let model: BoardModel;
  const items = new ItemSaves(status, {
    saved: (key, it) => model.saved(key, it),
    rejected: (key, e) => model.rejected(key, e),
  });
  model = new BoardModel({ items, reload, notify: (n) => notices.push(n) });
  return { model, calls, notices, reload, status, items };
}

const texts = (m: BoardModel) => m.todos.map((r) => r.text);
const sent = (calls: Call[]) => calls.map((c) => [c.cmd, c.args]);

test("読み込み直しても、まだ保存していない本文と、まだ作っていない行は画面の値を残す", async () => {
  const { model } = setup(() => {
    throw { kind: "storage", detail: "x" };
  });
  model.load(board([bi(item(A, "設計を読む")), bi(item(B, "レビュー"))]));
  model.editText(A, "設計書を読む");
  const f = model.add("todo", A)!;
  model.editText(f.key, "メモする");
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  // エージェントが B を変えて、C を足した
  model.load(board([bi(item(A, "設計を読む")), bi(item(B, "レビューする")), bi(item(C, "追加"))]));
  expect(texts(model)).toEqual(["設計書を読む", "メモする", "レビューする", "追加"]);
  // 別の日を読むと作り直す
  model.load(board([bi(item(C, "次の日"))], [], "2026-11-04"));
  expect(texts(model)).toEqual(["次の日"]);
});

test("足した行は本文が入ってから、いまの並びの位置に作り、足したことと入力は1回の取り消しで戻る", async () => {
  const { model, calls } = setup((cmd) => {
    if (cmd === "add_item") return item(NEW, "新しい");
    if (cmd === "delete_item") return { id: NEW, source: null };
    if (cmd === "restore_item") return item(NEW, "新しい");
    return null;
  });
  model.load(board([bi(item(A, "a")), bi(item(B, "b"))]));
  const f = model.add("todo", A)!;
  expect(texts(model)).toEqual(["a", "", "b"]);
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(calls).toEqual([]);
  model.editText(f.key, "新");
  model.editText(f.key, "新しい");
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(sent(calls)).toEqual([
    ["add_item", { date: DATE, kind: "todo", text: "新しい", position: { at: "after", id: A } }],
  ]);
  expect(model.row(f.key)!.id).toBe(NEW);
  await model.undo();
  await model.settled();
  expect(texts(model)).toEqual(["a", "b"]);
  expect(calls.at(-1)).toEqual({ cmd: "delete_item", args: { id: NEW } });
  await model.redo();
  expect(calls.at(-1)).toEqual({ cmd: "restore_item", args: { deleted: { id: NEW, source: null } } });
});

test("何も書かずに離れた行は、作らずに消す", async () => {
  const { model, calls } = setup();
  model.load(board([bi(item(A, "a"))]));
  const f = model.add("todo", null)!;
  model.leave(f.key);
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(texts(model)).toEqual(["a"]);
  expect(calls).toEqual([]);
});

test("完了の切り替えは状態を保存し、取り消すと元の完了時刻に戻す。その後にエージェントが変えていたら戻さない", async () => {
  const { model, calls } = setup(() => ({ updated: true, item: item(A, "a") }));
  model.load(board([bi(item(A, "a", { status: "done", completedAt: 5 }))]));
  model.toggle(A);
  expect(model.todos[0].status).toBe("open");
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(sent(calls)).toEqual([
    ["save_item", { id: A, fields: { status: { before: "done", closedAt: null, changedAt: Date.now() - SAVE_DELAY_MS } } }],
  ]);
  await model.undo();
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(model.todos[0]).toMatchObject({ status: "done", completedAt: 5 });
  expect((calls.at(-1)!.args.fields as { status: { closedAt: number } }).status.closedAt).toBe(5);
  // やり直した後、エージェントが完了に戻したら、取り消しても戻さない
  await model.redo();
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  model.load(board([bi(item(A, "a", { status: "done", completedAt: 9 }))]));
  const n = calls.length;
  await model.undo();
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(calls.length).toBe(n);
  expect(model.todos[0]).toMatchObject({ status: "done", completedAt: 9 });
});

test("障害を解消すると解消メモの欄を開き、未解消に戻すとメモを送ってから閉じる", async () => {
  const { model, calls } = setup(() => ({ updated: true, item: item(A, "a", { kind: "blocker" }) }));
  model.load(board([], [bi(item(A, "詰まり", { kind: "blocker" }))]));
  expect(model.toggle(A)).toEqual({ key: A, field: "memo", offset: "end" });
  expect(model.memoOpen).toBe(A);
  model.editMemo(A, "聞いた");
  model.toggle(A);
  expect(model.memoOpen).toBeNull();
  await vi.advanceTimersByTimeAsync(0);
  expect(calls[0].args.fields).toMatchObject({ resolutionNote: "聞いた", status: { before: "open" } });
  // 未解消の間は解消メモを出さない
  expect(showsMemo(model.blockers[0])).toBe(false);
});

test("持ち越し済みの項目と取りやめた TODO は、状態と本文を変えず、理由を知らせる", async () => {
  const { model, calls, notices } = setup();
  model.load(
    board([
      bi(item(A, "持ち越した"), { carriedTo: { id: C, date: "2026-11-04" } }),
      bi(item(B, "やめた", { status: "dropped", closeReason: "dropped" })),
    ]),
  );
  expect(lockOf(model.todos[0])).toEqual({ reason: "carried", to: { id: C, date: "2026-11-04" } });
  model.toggle(A);
  model.toggle(B);
  model.editText(A, "変える");
  expect(model.splitAt(B, 2)).toBeNull();
  expect(model.refuseEdit(A)).toBe(true);
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(calls).toEqual([]);
  expect(texts(model)).toEqual(["持ち越した", "やめた"]);
  expect(notices).toEqual([
    { kind: "carried", date: "2026-11-04" },
    { kind: "dropped" },
    { kind: "dropped" },
    { kind: "carried", date: "2026-11-04" },
  ]);
});

test("削除は取り消せ、持ち越し元が変わって戻せなければ理由を知らせる", async () => {
  let refuse = false;
  const { model, calls, notices } = setup((cmd) => {
    if (cmd === "delete_item") return { id: A, source: null };
    if (cmd === "restore_item") {
      if (refuse) throw { kind: "restore_refused", reason: "source_carried_over", carriedToId: C, carriedToDate: "2026-11-05" };
      return item(A, "a");
    }
    return null;
  });
  model.load(board([bi(item(A, "a")), bi(item(B, "b"))]));
  model.remove(A);
  await model.settled();
  expect(texts(model)).toEqual(["b"]);
  // 削除の前に始まった読み込みでも戻さない
  model.load(board([bi(item(A, "a")), bi(item(B, "b"))]));
  expect(texts(model)).toEqual(["b"]);
  await model.undo();
  expect(sent(calls).at(-1)).toEqual(["restore_item", { deleted: { id: A, source: null } }]);
  await model.redo();
  await model.settled();
  refuse = true;
  await model.undo();
  expect(notices).toEqual([{ kind: "restore_source_carried", date: "2026-11-05" }]);
});

test("並べ替えは、その項目の保存を送ってから、いまの並びの位置へ動かし、取り消すと元の位置に戻す", async () => {
  const { model, calls } = setup((cmd) => (cmd === "move_item" ? { updated: true, item: item(A, "a") } : null));
  model.load(board([bi(item(A, "a")), bi(item(B, "b")), bi(item(C, "c"))]));
  model.move(A, 1);
  expect(texts(model)).toEqual(["b", "a", "c"]);
  model.move(A, 1);
  model.move(A, 1);
  await model.settled();
  expect(texts(model)).toEqual(["b", "c", "a"]);
  // 位置は送る時点の並びで決める（続けて動かしても、最後の並びになる）。末尾から下へは動かさない
  expect(sent(calls)).toEqual([
    ["move_item", { id: A, position: { at: "after", id: C } }],
    ["move_item", { id: A, position: { at: "after", id: C } }],
  ]);
  await model.undo();
  await model.settled();
  expect(texts(model)).toEqual(["b", "a", "c"]);
  expect(sent(calls).at(-1)).toEqual(["move_item", { id: A, position: { at: "after", id: B } }]);
});

test("行の途中の Enter で2件に分け、行頭では直上、行末では直下に足す。取り消すとつなぎ直す", async () => {
  const { model, calls } = setup((cmd, args) => {
    if (cmd === "split_item") {
      return { original: item(A, args.first as string), created: item(NEW, args.second as string) };
    }
    if (cmd === "delete_item") return { id: NEW, source: null };
    return { updated: true, item: item(A, "a") };
  });
  model.load(board([bi(item(A, "設計を読む"))]));
  const f = model.splitAt(A, 2)!;
  expect(texts(model)).toEqual(["設計", "を読む"]);
  expect(f.offset).toBe(0);
  await vi.advanceTimersByTimeAsync(0);
  expect(sent(calls)).toEqual([["split_item", { id: A, first: "設計", second: "を読む" }]]);
  expect(model.row(f.key)!.id).toBe(NEW);
  await model.undo();
  await model.settled();
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(texts(model)).toEqual(["設計を読む"]);
  expect(sent(calls).slice(1)).toEqual([
    ["delete_item", { id: NEW }],
    ["save_item", { id: A, fields: { text: "設計を読む" } }],
  ]);
  // 行頭・行末
  model.splitAt(A, 0);
  model.splitAt(A, "設計を読む".length);
  expect(texts(model)).toEqual(["", "設計を読む", ""]);
});

test("分割の保存に失敗したら、2行のまま再試行する", async () => {
  let fail = true;
  const { model, calls, status } = setup((_cmd, args) => {
    if (fail) throw { kind: "storage", detail: "x" };
    return { original: item(A, args.first as string), created: item(NEW, args.second as string) };
  });
  model.load(board([bi(item(A, "設計を読む"))]));
  const f = model.splitAt(A, 2)!;
  await vi.advanceTimersByTimeAsync(0);
  expect(status.failing).toBe(true);
  model.editText(f.key, "を読み直す");
  // 読み込み直しても、2行のまま
  model.load(board([bi(item(A, "設計を読む"))]));
  expect(texts(model)).toEqual(["設計", "を読み直す"]);
  fail = false;
  await vi.advanceTimersByTimeAsync(RETRY_DELAYS_MS[1]);
  expect(sent(calls).at(-1)).toEqual(["split_item", { id: A, first: "設計", second: "を読み直す" }]);
  expect(status.failing).toBe(false);
});

test("入力中の欄は、フォーカスがあり最後の入力から10秒以内か変換中で、まだ保存していない欄も含む", async () => {
  const { model } = setup(() => {
    throw { kind: "storage", detail: "x" };
  });
  model.load(board([bi(item(A, "a")), bi(item(B, "b"))]));
  model.setFocus({ key: A, field: "text" });
  expect(model.editingItems(Date.now(), true)).toEqual([]);
  model.editText(A, "ab");
  expect(model.editingItems(Date.now(), true)).toEqual([{ itemId: A, field: "text" }]);
  // アプリが前面に無ければ、フォーカスの分は含めない（保存していない欄は含める）
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  model.setFocus({ key: B, field: "text" });
  model.editText(B, "b2");
  expect(model.editingItems(Date.now(), false)).toEqual([
    { itemId: A, field: "text" },
    { itemId: B, field: "text" },
  ]);
  model.setComposing(true);
  expect(model.editingUntil()).toBeNull();
  model.setComposing(false);
  expect(model.editingUntil()).toBe(Date.now() + EDITING_MS);
});

test("再試行しない失敗で捨てた保存は、理由を知らせて読み込み直す", async () => {
  const { model, notices, reload } = setup(() => {
    throw { kind: "conflict", reason: "carried_over", carriedToId: C, carriedToDate: "2026-11-04" };
  });
  model.load(board([bi(item(A, "a"))]));
  model.editText(A, "ab");
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(notices).toEqual([{ kind: "carried", date: "2026-11-04" }]);
  expect(reload).toHaveBeenCalled();
});

test("ドラッグで、同じ種類の中の好きな位置へ動かせ、取り消すと元の位置に戻す", async () => {
  const { model, calls } = setup((cmd) => (cmd === "move_item" ? { updated: true, item: item(C, "c") } : null));
  model.load(board([bi(item(A, "a")), bi(item(B, "b")), bi(item(C, "c"))]));
  model.moveTo(C, 0);
  await model.settled();
  expect(texts(model)).toEqual(["c", "a", "b"]);
  expect(sent(calls)).toEqual([["move_item", { id: C, position: { at: "before", id: A } }]]);
  await model.undo();
  await model.settled();
  expect(texts(model)).toEqual(["a", "b", "c"]);
});

test("作る前の行を分けても、新しい行の入力は新しい行に、元の行の入力は元の行に入る", async () => {
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  let created = 0;
  const { model } = setup((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "add_item") return item(created++ === 0 ? A : NEW, args.text as string);
    if (cmd === "split_item") {
      return { original: item(A, args.first as string), created: item(NEW, args.second as string) };
    }
    return { updated: true, item: item(args.id as string, "x") };
  });
  model.load(board([]));
  const f = model.add("todo", null)!;
  model.editText(f.key, "foo bar");
  const g = model.splitAt(f.key, 4)!;
  model.editText(g.key, "baz");
  model.editText(f.key, "fooo ");
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);
  expect(texts(model)).toEqual(["fooo ", "baz"]);
  // 分割を送っている間の入力は、分割の後に、それぞれの項目の保存として送る
  const saves = calls.filter((c) => c.cmd === "save_item").map((c) => c.args);
  expect(saves).toContainEqual({ id: A, fields: { text: "fooo " } });
  expect(saves).toContainEqual({ id: NEW, fields: { text: "baz" } });
  expect(saves.some((a) => a.id === A && (a.fields as { text?: string }).text === "baz")).toBe(false);
});

test("分割をやり直しても、やり直しの履歴は残る", async () => {
  const { model } = setup((cmd, args) => {
    if (cmd === "split_item") return { original: item(A, args.first as string), created: item(NEW, args.second as string) };
    if (cmd === "delete_item") return { id: NEW, source: null };
    return { updated: true, item: item(A, "a") };
  });
  model.load(board([bi(item(A, "設計を読む")), bi(item(B, "b"))]));
  model.splitAt(A, 2);
  await vi.advanceTimersByTimeAsync(0);
  model.setFocus({ key: B, field: "text" });
  model.editText(B, "bb");
  await model.undo();
  await model.undo();
  await model.settled();
  expect(texts(model)).toEqual(["設計を読む", "b"]);
  await model.redo();
  expect(texts(model)).toEqual(["設計", "を読む", "b"]);
  expect(model.canRedo).toBe(true);
  await model.redo();
  expect(texts(model)).toEqual(["設計", "を読む", "bb"]);
});

test("送り直しても成功しない失敗で分割を捨てたら、2行の本文をつないで元の項目に送る", async () => {
  const { model, calls } = setup((cmd) => {
    if (cmd === "split_item") throw { kind: "internal", detail: "x" };
    return { updated: true, item: item(A, "a") };
  });
  model.load(board([bi(item(A, "設計を読む"))]));
  model.splitAt(A, 2);
  await vi.advanceTimersByTimeAsync(0);
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  expect(texts(model)).toEqual(["設計を読む"]);
  expect(sent(calls).at(-1)).toEqual(["save_item", { id: A, fields: { text: "設計を読む" } }]);
});

test("削除を送り終える前に取り消しても、送り終えてから戻す", async () => {
  let release: () => void = () => {};
  const { model, calls } = setup(async (cmd) => {
    if (cmd === "delete_item") {
      await new Promise<void>((r) => (release = r));
      return { id: A, source: null };
    }
    if (cmd === "restore_item") return item(A, "a");
    return null;
  });
  model.load(board([bi(item(A, "a"))]));
  model.remove(A);
  const undo = model.undo();
  await vi.advanceTimersByTimeAsync(0);
  release();
  await undo;
  expect(sent(calls).map((c) => c[0])).toEqual(["delete_item", "restore_item"]);
});

test("画面で作った行が、読み込みに2回続けて出てこなければ、ほかで消されたものとして消す", async () => {
  const { model } = setup(() => item(NEW, "新しい"));
  model.load(board([bi(item(A, "a"))]));
  const f = model.add("todo", A)!;
  model.editText(f.key, "新しい");
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  // 作る前に始まった読み込み（1回目）では残す
  model.load(board([bi(item(A, "a"))]));
  expect(texts(model)).toEqual(["a", "新しい"]);
  model.load(board([bi(item(A, "a"))]));
  expect(texts(model)).toEqual(["a"]);
});

test("足して入力した行を、送る前に取り消したら、作らない", async () => {
  const { model, calls } = setup(() => item(NEW, "abc"));
  model.load(board([bi(item(A, "a"))]));
  const f = model.add("todo", A)!;
  model.editText(f.key, "abc");
  await model.undo();
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS * 2);
  expect(texts(model)).toEqual(["a"]);
  expect(calls).toEqual([]);
});

test("再試行を待つ分割を取り消しても、失敗の知らせを出さない", async () => {
  const { model, notices, calls } = setup((cmd) => {
    if (cmd === "split_item") throw { kind: "storage", detail: "x" };
    return { updated: true, item: item(A, "a") };
  });
  model.load(board([bi(item(A, "設計を読む"))]));
  model.splitAt(A, 2);
  await vi.advanceTimersByTimeAsync(0);
  await model.undo();
  await vi.advanceTimersByTimeAsync(RETRY_DELAYS_MS[0] * 2);
  expect(notices).toEqual([]);
  expect(texts(model)).toEqual(["設計を読む"]);
  expect(calls.filter((c) => c.cmd === "split_item").length).toBe(1);
});

test("本文を消して離れた項目は、消し始める前の本文で削除し、取り消すとその本文で戻る", async () => {
  const { model, calls } = setup((cmd) => {
    if (cmd === "delete_item") return { id: A, source: null };
    if (cmd === "restore_item") return item(A, "Buy milk");
    return { updated: true, item: item(A, "x") };
  });
  model.load(board([bi(item(A, "Buy milk"))]));
  model.setFocus({ key: A, field: "text" });
  model.editText(A, "B");
  model.editText(A, "");
  model.leave(A);
  await model.settled();
  // 途中の「B」は送らず、消し始める前の本文に戻してから削除する
  expect(sent(calls)).toEqual([
    ["save_item", { id: A, fields: { text: "Buy milk" } }],
    ["delete_item", { id: A }],
  ]);
  await model.undo();
  expect(sent(calls).at(-1)![0]).toBe("restore_item");
});

test("作っている途中に届いた読み込みで、同じ行を二重に出さない", async () => {
  let release: () => void = () => {};
  const { model } = setup(async () => {
    await new Promise<void>((r) => (release = r));
    return item(NEW, "新しい");
  });
  model.load(board([bi(item(A, "a"))]));
  const f = model.add("todo", A)!;
  model.editText(f.key, "新しい");
  await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
  // 作り終えた後の読み込みが、応答より先に届いた
  model.load(board([bi(item(A, "a")), bi(item(NEW, "新しい"))]));
  expect(texts(model)).toEqual(["a", "新しい"]);
  release();
  await vi.advanceTimersByTimeAsync(0);
  model.load(board([bi(item(A, "a")), bi(item(NEW, "新しい"))]));
  expect(model.todos.map((r) => r.key)).toEqual([A, f.key]);
});
