// 凪メーターの景色と文字の中身（画面設計書 3章「凪メーター」、デザインガイド 1章）。
// 表示（NagiMeter.svelte）から分けて、件数と時刻だけで決まるようにする。
//
// - 太陽：いまの時刻。始業から終業まで、左から右へ弧を描いて進む。終業を過ぎると水平線に半分沈む。
//   過去の日は終業の位置、未来の日は始業の位置。
// - 波：残りの仕事（未完了の TODO と未解消の障害の、全体に対する割合）。0 件で平らな線（凪）。
// - 光の道：終えた仕事（完了・解消の割合）。大きいほど、太陽の下の海面の光の線が多い。

import type { Counts, LogicalDate } from "./bindings";
import { calendarDate, daysBetween } from "./dates";

/** 凪メーターの時間帯（分。設定「凪メーターの時間帯」の既定。画面設計書 7章）。 */
export const WORK_START_MIN = 9 * 60;
export const WORK_END_MIN = 19 * 60;

/** 時刻の文字の種類。 */
export type MeterTime = { kind: "now"; minutes: number } | { kind: "dayEnd" } | { kind: "notYet" };

export interface MeterScene {
  time: MeterTime;
  /** 太陽の進み（0 が始業、1 が終業）。 */
  progress: number;
  /** 終業を過ぎて、水平線に半分沈んでいる。 */
  sunk: boolean;
  /** 残り（未完了の TODO と未解消の障害）。 */
  left: number;
  /** TODO と障害の合計（取りやめた TODO は除く）。 */
  total: number;
  /** 残りの割合（0〜1）。 */
  leftRatio: number;
  /** 項目があり、残りが 0 件（凪）。 */
  calm: boolean;
  /** 光の道の本数。 */
  paths: number;
}

/**
 * いまの時刻の、その論理日付の始まりからの分。論理日付の切り替わり時刻（既定 4:00）より前の深夜は、
 * まだ前の日なので、暦の日が論理日付より後なら 24 時間ずつ足す（終業を過ぎた位置になる）。
 */
export function minutesOfDay(today: LogicalDate, now: Date): number {
  const extraDays = Math.max(0, daysBetween(today, calendarDate(now)));
  return extraDays * 24 * 60 + now.getHours() * 60 + now.getMinutes();
}

/**
 * 表示している日 `date` の凪メーターの中身。`today` は今日の論理日付、`minutes` は今日の
 * いまの時刻（`minutesOfDay`）。
 */
export function meterScene(
  date: LogicalDate,
  today: LogicalDate,
  minutes: number,
  counts: Counts,
  hours: { start: number; end: number } = { start: WORK_START_MIN, end: WORK_END_MIN },
): MeterScene {
  const time: MeterTime =
    date < today ? { kind: "dayEnd" } : date > today ? { kind: "notYet" } : { kind: "now", minutes };
  const at = time.kind === "dayEnd" ? hours.end : time.kind === "notYet" ? hours.start : minutes;
  const progress = Math.min(1, Math.max(0, (at - hours.start) / (hours.end - hours.start)));
  const left = counts.leftTodos + counts.leftBlockers;
  const total = counts.todos + counts.leftBlockers + counts.resolved;
  const closed = total - left;
  return {
    time,
    progress,
    sunk: at >= hours.end,
    left,
    total,
    leftRatio: total === 0 ? 0 : left / total,
    calm: total > 0 && left === 0,
    // 終えたものがあれば 2 本から、すべて終えると 8 本
    paths: total === 0 || closed === 0 ? 0 : Math.round(2 + (6 * closed) / total),
  };
}

/** 時刻の表示（`16:42`）。 */
export function hhmm(minutes: number): string {
  const m = ((minutes % 1440) + 1440) % 1440;
  return `${Math.floor(m / 60)}:${String(m % 60).padStart(2, "0")}`;
}

/** 波の線（SVG のパス）：幅 `w`、基準の高さ `y0`、振幅 `amp`、波の数 `k`、位相 `phase`。 */
export function wavePath(w: number, y0: number, amp: number, k: number, phase: number): string {
  const pts: string[] = [];
  for (let i = 0; i <= 100; i++) {
    const x = (w * i) / 100;
    const y = y0 + amp * Math.sin((i / 100) * Math.PI * 2 * k + phase);
    pts.push(`${i === 0 ? "M" : "L"}${x.toFixed(1)} ${y.toFixed(2)}`);
  }
  return pts.join(" ");
}
