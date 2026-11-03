// 論理日付（`YYYY-MM-DD`）の計算と表示（画面設計書 3章「上部のバー」）。
// 論理日付は時刻を持たない暦の日なので、端末のタイムゾーンに左右されないよう UTC の日として計算する。

import type { LogicalDate } from "./bindings";

/** 曜日の英字ラベル（日曜から。日付の表示は日本語の画面でも英語。デザインガイド 7章）。 */
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"] as const;

function parse(date: LogicalDate): Date {
  const [y, m, d] = date.split("-").map(Number);
  return new Date(Date.UTC(y, m - 1, d));
}

function format(d: Date): LogicalDate {
  return d.toISOString().slice(0, 10);
}

/** `date` の `days` 日後（負なら前）。 */
export function addDays(date: LogicalDate, days: number): LogicalDate {
  const d = parse(date);
  d.setUTCDate(d.getUTCDate() + days);
  return format(d);
}

/** 曜日の英字ラベル（`Thu`）。 */
export function weekday(date: LogicalDate): string {
  return WEEKDAYS[parse(date).getUTCDay()];
}

/** 月と日（`10.29`）。年は出さない。 */
export function monthDay(date: LogicalDate): string {
  return date.slice(5).replace("-", ".");
}

/** 日付の表示（`10.29 Thu`）。 */
export function shortDate(date: LogicalDate): string {
  return `${monthDay(date)} ${weekday(date)}`;
}

/** 端末の時刻 `now` の暦の日（端末のタイムゾーン）。 */
export function calendarDate(now: Date): LogicalDate {
  const y = now.getFullYear();
  const m = String(now.getMonth() + 1).padStart(2, "0");
  const d = String(now.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

/** `a` から `b` までの日数（`b` が後なら正）。 */
export function daysBetween(a: LogicalDate, b: LogicalDate): number {
  return Math.round((parse(b).getTime() - parse(a).getTime()) / 86_400_000);
}
