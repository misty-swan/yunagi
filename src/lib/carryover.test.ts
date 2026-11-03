import { expect, test } from "vitest";
import type { CarryoverList, Item } from "./bindings";
import { decisionOf, mergeRows, splitChildren, type CarryRow } from "./carryover.svelte";

test("分割の子は LF・CRLF・CR のどれでも行に分ける", () => {
  expect(splitChildren("a\nb\r\nc\rd")).toEqual(["a", "b", "c", "d"]);
});

function item(id: string, text: string, over: Partial<Item> = {}): Item {
  return {
    id,
    date: "2026-11-02",
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

const row = (i: Item, over: Partial<CarryRow> = {}): CarryRow => ({
  item: i,
  review: false,
  checked: true,
  choice: null,
  children: "",
  ...over,
});

const list = (items: { item: Item; review?: boolean }[], from = "2026-11-02"): CarryoverList => ({
  from,
  items: items.map((c) => ({ item: c.item, review: c.review ?? false })),
  reviewThreshold: 3,
});

test("開いたときは全項目にチェックを付ける", () => {
  const rows = mergeRows([], list([{ item: item("A", "a") }, { item: item("B", "b", { kind: "blocker" }) }]), true);
  expect(rows.map((r) => [r.item.id, r.checked, r.choice])).toEqual([
    ["A", true, null],
    ["B", true, null],
  ]);
});

test("読み直すと選択を引き継ぎ、消えた項目は捨て、加わった項目はチェックを付けずに加える", () => {
  const before = [
    row(item("A", "a"), { checked: false }),
    row(item("B", "b", { carryCount: 3 }), { review: true, checked: false, choice: "split", children: "x\ny" }),
    row(item("C", "c")),
  ];
  const after = mergeRows(
    before,
    list([
      { item: item("B", "b 変えた", { carryCount: 3 }), review: true },
      { item: item("A", "a") },
      { item: item("D", "足した") },
    ]),
    false,
  );
  expect(after.map((r) => [r.item.id, r.item.text, r.checked, r.choice, r.children])).toEqual([
    ["B", "b 変えた", false, "split", "x\ny"],
    ["A", "a", false, null, ""],
    ["D", "足した", false, null, ""],
  ]);
  // 消えた C がもう一度加わっても（未完了に戻した）、チェックは付けない
  const back = mergeRows(after, list([{ item: item("C", "c") }]), false);
  expect(back[0].checked).toBe(false);
});

test("見直しの選択肢が出なくなったら、選んでいた見直しを捨てて持ち越す側に戻す", () => {
  const before = [row(item("A", "a"), { review: true, checked: false, choice: "drop" })];
  const after = mergeRows(before, list([{ item: item("A", "a"), review: false }]), false);
  expect(after[0]).toMatchObject({ checked: true, choice: null, children: "" });
});

test("ボタンごとの確定の中身", () => {
  const rows = [
    row(item("A", "a")),
    row(item("B", "b"), { checked: false }),
    row(item("C", "c"), { review: true, checked: false, choice: "drop" }),
    row(item("D", "d"), { review: true, checked: false, choice: "split", children: "前半\r\n\n後半" }),
    row(item("E", "e"), { review: true, checked: false, choice: "split", children: "" }),
  ];
  const reviews = [
    { id: "C", review: { review: "drop" } },
    { id: "D", review: { review: "split", children: ["前半", "", "後半"] } },
    { id: "E", review: { review: "split", children: [""] } },
  ];
  // 子の無い分割（E）は、見直しを選ばなかったものとして持ち越す
  expect(decisionOf(rows, "selected")).toEqual({ selection: { select: "selected", ids: ["A", "D", "E"] }, reviews });
  expect(decisionOf(rows, "all")).toEqual({ selection: { select: "all" }, reviews });
  expect(decisionOf(rows, "none")).toEqual({ selection: { select: "none" }, reviews });
  // 見直しの選択肢を添えない行の選択は送らない
  expect(decisionOf([row(item("F", "f"), { choice: "drop" })], "all").reviews).toEqual([]);
});
