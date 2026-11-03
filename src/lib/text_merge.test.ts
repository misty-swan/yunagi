import { expect, test } from "vitest";
import { diffLines, mergeText, unsavedLines } from "./text_merge";

const join = (...lines: string[]) => lines.join("\n");

test("diffLines は、もとの行の範囲と置き換える行を返す", () => {
  expect(diffLines(["a", "b", "c"], ["a", "b", "c"])).toEqual([]);
  expect(diffLines(["a", "b", "c"], ["a", "B", "c"])).toEqual([{ start: 1, end: 2, lines: ["B"] }]);
  expect(diffLines(["a", "c"], ["a", "b", "c"])).toEqual([{ start: 1, end: 1, lines: ["b"] }]);
  expect(diffLines(["a", "b", "c"], ["b", "c", "a"])).toEqual([
    { start: 0, end: 1, lines: [] },
    { start: 3, end: 3, lines: ["a"] },
  ]);
  expect(diffLines([""], ["x"])).toEqual([{ start: 0, end: 1, lines: ["x"] }]);
});

test("重ならない変更は両方を残す", () => {
  const base = join("a", "b", "c", "d");
  expect(mergeText(base, join("A", "b", "c", "d"), join("a", "b", "c", "D"))).toBe(
    join("A", "b", "c", "D"),
  );
  // 片方だけが変えたなら、その結果
  expect(mergeText(base, base, join("a", "x"))).toBe(join("a", "x"));
  expect(mergeText(base, join("a", "x"), base)).toBe(join("a", "x"));
});

test("同じ位置への追加は両方を残し、利用者の行が先", () => {
  // エージェントの追記（末尾）と、利用者が末尾に足した行
  expect(mergeText(join("a", "b"), join("a", "b", "agent"), join("a", "b", "mine"))).toBe(
    join("a", "b", "mine", "agent"),
  );
});

test("利用者の新しい行は、すぐ上の行がエージェントの変更の範囲に入ったら、範囲の位置に残す", () => {
  const base = join("a", "b", "c");
  const mine = join("a", "b", "mine", "c");
  // 書き換え：新しい行の後ろ
  expect(mergeText(base, join("a", "B1", "B2", "c"), mine)).toBe(join("a", "B1", "B2", "mine", "c"));
  // 削除：範囲のすぐ上の行の後ろ
  expect(mergeText(base, join("a", "c"), mine)).toBe(join("a", "mine", "c"));
  // 移動（d を先頭へ）：利用者の行は一緒に動かさず、範囲のすぐ上の行の後ろに残す
  expect(mergeText(join("a", "b", "c", "d"), join("d", "a", "b", "c"), join("a", "b", "c", "d", "mine"))).toBe(
    join("d", "a", "b", "c", "mine"),
  );
});

test("同じ行を両方が変えたら、利用者の結果、続けてエージェントの結果を置く", () => {
  const base = join("a", "b", "c");
  expect(mergeText(base, join("a", "agent", "c"), join("a", "mine", "c"))).toBe(
    join("a", "mine", "agent", "c"),
  );
  // 片方が削除なら、その側の行は無い
  expect(mergeText(base, join("a", "c"), join("a", "mine", "c"))).toBe(join("a", "mine", "c"));
  // 同じ結果なら1つだけ
  expect(mergeText(base, join("a", "B", "c"), join("a", "B", "c", "d"))).toBe(join("a", "B", "c", "d"));
});

test("エージェントの新しい行が、利用者の書き換えた行に付いているときは、その後ろに置く", () => {
  const base = join("a", "b", "c");
  expect(mergeText(base, join("a", "b", "agent", "c"), join("a", "mine", "c"))).toBe(
    join("a", "mine", "agent", "c"),
  );
});

test("unsavedLines は、もとの本文で数えた、書き換え・削除した行", () => {
  const base = join("a", "b", "c", "d");
  expect(unsavedLines(base, base)).toEqual([]);
  // 書き換え（差分の上では削除＋追加）も、元の行として数える
  expect(unsavedLines(base, join("a", "B", "c", "d"))).toEqual([2]);
  expect(unsavedLines(base, join("a", "d"))).toEqual([2, 3]);
  // 足しただけの行は知らせない
  expect(unsavedLines(base, join("a", "x", "b", "c", "d"))).toEqual([]);
  expect(unsavedLines(base, join("x", "a", "b", "c", "D", "y"))).toEqual([4]);
});
