import { expect, test } from "vitest";
import { parseDateQuery } from "./date_query";
import { dateOf, longDate } from "./dates";

const TODAY = "2026-11-03";

test("年を含む日付を、区切りによらず読む（全角・曜日付きも）", () => {
  for (const q of ["2026-10-31", "2026/10/31", "2026.10.31", " 2026.10.31 Sat ", "２０２６－１０－３１", "2026年10月31日"]) {
    expect(parseDateQuery(q, TODAY), q).toBe("2026-10-31");
  }
  expect(parseDateQuery("2025-1-5", TODAY)).toBe("2025-01-05");
});

test("年の無い日付は、今日の年にする", () => {
  expect(parseDateQuery("10/31", TODAY)).toBe("2026-10-31");
  expect(parseDateQuery("1-5", TODAY)).toBe("2026-01-05");
  // 区切りが . のときは、上部のバーの表示の形（曜日付き）だけ
  expect(parseDateQuery("10.29 Thu", TODAY)).toBe("2026-10-29");
  expect(parseDateQuery("1.2", TODAY)).toBeNull();
  expect(parseDateQuery("3.14", TODAY)).toBeNull();
  expect(parseDateQuery("12月24日", TODAY)).toBe("2026-12-24");
});

test("相対的な日付は今日（論理日付）を基準にする", () => {
  expect(parseDateQuery("yesterday", TODAY)).toBe("2026-11-02");
  expect(parseDateQuery("Yesterday", TODAY)).toBe("2026-11-02");
  expect(parseDateQuery("昨日", TODAY)).toBe("2026-11-02");
  expect(parseDateQuery("today", TODAY)).toBe(TODAY);
  expect(parseDateQuery("明日", TODAY)).toBe("2026-11-04");
  expect(parseDateQuery("一昨日", TODAY)).toBe("2026-11-01");
  expect(parseDateQuery("あさって", TODAY)).toBe("2026-11-05");
  // 月や年をまたぐ
  expect(parseDateQuery("昨日", "2026-01-01")).toBe("2025-12-31");
});

test("暦に無い日や、日付でない入力は日付として扱わない", () => {
  for (const q of ["", "  ", "2026-02-30", "2026-13-01", "10/32", "定例", "2026", "v1.2.3", "昨日の定例", "2026-10-31x"]) {
    expect(parseDateQuery(q, TODAY), q).toBeNull();
  }
  expect(parseDateQuery("2028-02-29", TODAY)).toBe("2028-02-29");
});

test("年を含む日付の表示と、暦の日の確かめ", () => {
  expect(longDate("2026-10-28")).toBe("2026.10.28 Wed");
  expect(dateOf(2026, 2, 29)).toBeNull();
  expect(dateOf(2024, 2, 29)).toBe("2024-02-29");
  expect(dateOf(0, 1, 1)).toBeNull();
});
