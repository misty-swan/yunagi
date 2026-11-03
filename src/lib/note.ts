// ノートの本文の扱いのうち、画面だけが使うもの（見た目は持たない）。
//
// - 時刻付き見出しの判定は、コアの `domain::note_body`（データモデル設計書 3章 note）と同じ形：
//   行頭の `## ` に続けて時刻（`H:MM` か `HH:MM`。0:00〜23:59）、その後は行末か、空白（全角の空白を
//   含む）に続く件名。
// - 時刻付き見出しの挿入（Mod+Shift+H）と、「- 」の行の Enter（操作仕様書 2章「ノートの編集」）。

/** 時刻付き見出しの行の中身。位置は行の先頭からの文字の数。 */
export interface TimeHeading {
  /** 時刻の文字（書いたままの形。例：`9:30`）。 */
  time: string;
  /** 件名（前後の空白を除く。無ければ空）。 */
  title: string;
  /** 時刻の始まりと終わり（`## ` の後ろから）。 */
  timeFrom: number;
  timeTo: number;
}

const TIME_HEADING = /^## (\d{1,2}):(\d{2})(?=$|\s)/u;

/** 1行が時刻付き見出しなら、その中身。そうでなければ `null`。 */
export function timeHeadingOf(line: string): TimeHeading | null {
  const m = TIME_HEADING.exec(line);
  if (m === null) return null;
  if (Number(m[1]) > 23 || Number(m[2]) > 59) return null;
  const timeTo = m[0].length;
  return { time: line.slice(3, timeTo), title: line.slice(timeTo).trim(), timeFrom: 3, timeTo };
}

/** 時刻 `now` の見出しの行の始め（`## HH:MM `。続けて件名を書けるよう、空白で終える）。 */
export function timeHeadingPrefix(now: Date): string {
  const hh = String(now.getHours()).padStart(2, "0");
  const mm = String(now.getMinutes()).padStart(2, "0");
  return `## ${hh}:${mm} `;
}

/** 本文への1つの書き換えと、その後のカーソルの位置。 */
export interface Edit {
  from: number;
  to: number;
  insert: string;
  caret: number;
}

/**
 * 時刻付き見出しを入れる書き換え：カーソルのある行（`line`。`from` はその行の始まりの位置）が空なら
 * その行に、そうでなければ空行を1行あけて次の行に入れる。カーソルは見出しの後ろ（件名を書く所）。
 * 空白だけの行も空とみなし、その空白は見出しに置き換える。
 */
export function insertTimeHeading(line: { from: number; text: string }, now: Date): Edit {
  const head = timeHeadingPrefix(now);
  const to = line.from + line.text.length;
  if (line.text.trim() === "") return { from: line.from, to, insert: head, caret: line.from + head.length };
  const insert = `\n\n${head}`;
  return { from: to, to, insert, caret: to + insert.length };
}

const LIST_ITEM = /^(\s*)- /;

/**
 * 「- 」で始まる行の Enter：次の行も同じ字下げの「- 」で始める。中身のない「- 」の行では「- 」だけを
 * 消し、字下げは残す（字下げの無い行は空の行になる）。
 * カーソル（`at`。行の始まりからの文字の数）が「- 」より前にあるときと、リストの行でないときは `null`
 * （ふつうの Enter）。
 */
export function continueList(line: { from: number; text: string }, at: number): Edit | null {
  const m = LIST_ITEM.exec(line.text);
  if (m === null || at < m[0].length) return null;
  if (line.text.trim() === "-") {
    const indent = m[1].length;
    return { from: line.from + indent, to: line.from + line.text.length, insert: "", caret: line.from + indent };
  }
  const insert = `\n${m[1]}- `;
  return { from: line.from + at, to: line.from + at, insert, caret: line.from + at + insert.length };
}
