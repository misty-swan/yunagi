import { expect, test } from "vitest";
import type { Counts } from "./bindings";
import { hhmm, meterScene, minutesOfDay, progressOf, WORK_END_MIN, WORK_START_MIN } from "./meter";

const counts = (c: Partial<Counts>): Counts => ({
  todos: 0,
  done: 0,
  leftTodos: 0,
  leftBlockers: 0,
  resolved: 0,
  carried: 0,
  ...c,
});

const TODAY = "2026-10-29";

test("太陽：今日はいまの時刻、過去の日は終業、未来の日は始業", () => {
  const c = counts({ todos: 1, leftTodos: 1 });
  const noon = meterScene(TODAY, TODAY, 14 * 60, c);
  expect(noon.time).toEqual({ kind: "now", minutes: 14 * 60 });
  expect(noon.progress).toBeCloseTo(0.5);
  expect(noon.sunk).toBe(false);
  // 始業前は始業の位置、終業を過ぎると沈む
  expect(meterScene(TODAY, TODAY, 7 * 60, c).progress).toBe(0);
  const late = meterScene(TODAY, TODAY, 20 * 60, c);
  expect(late.progress).toBe(1);
  expect(late.sunk).toBe(true);
  const past = meterScene("2026-10-28", TODAY, 10 * 60, c);
  expect(past.time).toEqual({ kind: "dayEnd" });
  expect(past.progress).toBe(1);
  expect(past.sunk).toBe(true);
  const future = meterScene("2026-10-30", TODAY, 10 * 60, c);
  expect(future.time).toEqual({ kind: "notYet" });
  expect(future.progress).toBe(0);
  // 時間帯を変えられる
  expect(meterScene(TODAY, TODAY, 12 * 60, c, { start: 10 * 60, end: 14 * 60 }).progress).toBe(0.5);
  expect(WORK_END_MIN - WORK_START_MIN).toBe(600);
});

test("波は残りの割合、光の道は終えた割合で決まる", () => {
  // TODO 9（完了 3）、障害 3（未解消 3）
  const s = meterScene(TODAY, TODAY, 600, counts({ todos: 9, done: 3, leftTodos: 6, leftBlockers: 3 }));
  expect(s.left).toBe(9);
  expect(s.total).toBe(12);
  expect(s.leftRatio).toBe(0.75);
  expect(s.calm).toBe(false);
  expect(s.paths).toBe(4);
  // すべて終えた日は凪
  const calm = meterScene(TODAY, TODAY, 600, counts({ todos: 2, done: 2, resolved: 1 }));
  expect(calm.calm).toBe(true);
  expect(calm.paths).toBe(8);
  // 項目の無い日は凪にしない（波も光の道も無い）
  const empty = meterScene(TODAY, TODAY, 600, counts({}));
  expect(empty.calm).toBe(false);
  expect(empty.leftRatio).toBe(0);
  expect(empty.paths).toBe(0);
  // 何も終えていなければ光の道は無い
  expect(meterScene(TODAY, TODAY, 600, counts({ todos: 1, leftTodos: 1 })).paths).toBe(0);
});

test("論理日付の切り替わり前の深夜は、前の日の続きの時刻にする", () => {
  expect(minutesOfDay(TODAY, new Date(2026, 9, 29, 16, 42))).toBe(16 * 60 + 42);
  expect(minutesOfDay(TODAY, new Date(2026, 9, 30, 1, 30))).toBe(24 * 60 + 90);
  expect(hhmm(16 * 60 + 42)).toBe("16:42");
  expect(hhmm(24 * 60 + 90)).toBe("1:30");
});

test("終業が始業より後でない時間帯でも、太陽の位置が決まる", () => {
  const c = counts({ todos: 1, leftTodos: 1 });
  const same = { start: 9 * 60, end: 9 * 60 };
  expect(meterScene(TODAY, TODAY, 8 * 60, c, same).progress).toBe(0);
  const after = meterScene(TODAY, TODAY, 10 * 60, c, same);
  expect(after.progress).toBe(1);
  expect(after.sunk).toBe(true);
  expect(Number.isNaN(meterScene(TODAY, TODAY, 10 * 60, c, { start: 600, end: 300 }).progress)).toBe(false);
});

test("残りの様子は凪メーターとサイドバーで同じ数え方", () => {
  const c = counts({ todos: 3, done: 1, leftTodos: 2, leftBlockers: 1, resolved: 1 });
  expect(progressOf(c)).toEqual({ left: 3, total: 5, leftRatio: 0.6, calm: false });
  const s = meterScene(TODAY, TODAY, 600, c);
  expect([s.left, s.total, s.leftRatio, s.calm]).toEqual([3, 5, 0.6, false]);
});
