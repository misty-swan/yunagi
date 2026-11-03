import { expect, test } from "vitest";
import { addDays, calendarDate, daysBetween, monthDay, shortDate, weekday } from "./dates";

test("日付を 10.29 Thu の形で出す", () => {
  expect(shortDate("2026-10-29")).toBe("10.29 Thu");
  expect(monthDay("2026-01-05")).toBe("01.05");
  expect(weekday("2026-11-01")).toBe("Sun");
});

test("日を足し引きする（月・年をまたぐ）", () => {
  expect(addDays("2026-10-31", 1)).toBe("2026-11-01");
  expect(addDays("2027-01-01", -1)).toBe("2026-12-31");
  expect(addDays("2028-02-28", 1)).toBe("2028-02-29");
  expect(daysBetween("2026-10-31", "2026-11-02")).toBe(2);
  expect(daysBetween("2026-11-02", "2026-10-31")).toBe(-2);
});

test("端末の時刻の暦の日", () => {
  expect(calendarDate(new Date(2026, 10, 3, 23, 59))).toBe("2026-11-03");
  expect(calendarDate(new Date(2026, 0, 9, 0, 0))).toBe("2026-01-09");
});
