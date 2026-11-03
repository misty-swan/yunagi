// パレットの日付ジャンプ（画面設計書 6章）：入力が日付なら、その論理日付を返す。
//
// 受け付ける形（前後の空白は除く。全角の数字・記号も半角として読む）：
// - 年を含む日付：`2026-10-31`・`2026/10/31`・`2026.10.31`（月と日は1桁でもよい）、`2026年10月31日`。
//   後ろに曜日の英字（`2026.10.28 Wed`。パレットの表示の形）が付いていてもよい（曜日は読まない）。
// - 年の無い日付：`10-31`・`10/31`・`10月31日`と、上部のバーの表示の形 `10.29 Thu`（区切りが `.` の
//   ときは曜日の英字が要る。`1.2`・`3.14` のような版の番号や小数を、検索せずに日付として選ばないため）。
//   年は今日（論理日付）の年にする。
// - 相対的な日付：today・yesterday・tomorrow（大文字・小文字は問わない）、今日・昨日・明日・一昨日・明後日
//   （かなの きょう・きのう・あした・おととい・あさって も）。今日（論理日付）を基準にする。
// 暦に無い日（`2026-02-30` など）は日付として扱わない。

import type { LogicalDate } from "./bindings";
import { addDays, dateOf } from "./dates";

/** 相対的な日付の語と、今日からの日数。 */
const RELATIVE = new Map<string, number>([
  ["today", 0],
  ["yesterday", -1],
  ["tomorrow", 1],
  ["今日", 0],
  ["きょう", 0],
  ["本日", 0],
  ["昨日", -1],
  ["きのう", -1],
  ["明日", 1],
  ["あした", 1],
  ["あす", 1],
  ["一昨日", -2],
  ["おととい", -2],
  ["明後日", 2],
  ["あさって", 2],
]);

/** 区切りが `-` `/` `.` の日付（年は任意）。後ろの曜日の英字は読み捨てる。 */
const NUMERIC = /^(?:(\d{4})[-/.])?(\d{1,2})[-/.](\d{1,2})(?:\s+(?:sun|mon|tue|wed|thu|fri|sat))?$/i;
/** `2026年10月31日`・`10月31日`。 */
const KANJI = /^(?:(\d{4})年)?(\d{1,2})月(\d{1,2})日$/;

/** 入力 `text` が日付なら、その論理日付（`today` は今日の論理日付）。日付でなければ `null`。 */
export function parseDateQuery(text: string, today: LogicalDate): LogicalDate | null {
  const q = text.normalize("NFKC").trim();
  if (q === "") return null;
  const rel = RELATIVE.get(q.toLowerCase());
  if (rel !== undefined) return addDays(today, rel);
  const m = NUMERIC.exec(q) ?? KANJI.exec(q);
  if (m === null) return null;
  if (m[1] === undefined && /^\d{1,2}\.\d{1,2}$/.test(q)) return null;
  const year = m[1] === undefined ? Number(today.slice(0, 4)) : Number(m[1]);
  return dateOf(year, Number(m[2]), Number(m[3]));
}
