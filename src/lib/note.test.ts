import { expect, test } from "vitest";
import { blank, continueList, insertTimeHeading, timeHeadingOf, timeHeadingPrefix } from "./note";

test("時刻付き見出しの判定はコアと同じ形（## の後に H:MM か HH:MM、その後は行末か空白）", () => {
  expect(timeHeadingOf("## 10:00 定例")).toEqual({ time: "10:00", title: "定例", timeFrom: 3, timeTo: 8 });
  expect(timeHeadingOf("## 9:30")).toEqual({ time: "9:30", title: "", timeFrom: 3, timeTo: 7 });
  expect(timeHeadingOf("## 23:59　振り返り")?.title).toBe("振り返り");
  expect(timeHeadingOf("## 10:00 ")?.title).toBe("");
  // 空白はコア（Rust の char::is_whitespace）と同じ：U+0085 は空白、U+FEFF は空白でない
  expect(timeHeadingOf("## 10:00\u0085件名\u0085")?.title).toBe("件名");
  expect(timeHeadingOf("## 10:00\uFEFF件名")).toBeNull();
  for (const no of ["## 良かったこと", "### 10:00", "##10:00", "## 24:00", "## 10:60", "## 10:0", "## 100:00", "## 10:00定例", " ## 10:00"]) {
    expect(timeHeadingOf(no)).toBeNull();
  }
});

test("時刻付き見出しは、空の行ならその行に、そうでなければ空行を1行あけて次の行に入れる", () => {
  const now = new Date(2026, 10, 3, 9, 5);
  expect(timeHeadingPrefix(now)).toBe("## 09:05 ");
  expect(insertTimeHeading({ from: 10, text: "" }, now)).toEqual({ from: 10, to: 10, insert: "## 09:05 ", caret: 19 });
  // 空白だけの行も空とみなし、空白は見出しに置き換える
  expect(insertTimeHeading({ from: 0, text: "  " }, now)).toEqual({ from: 0, to: 2, insert: "## 09:05 ", caret: 9 });
  // CR だけの行は空とみなさない（CR を消さない）
  expect(insertTimeHeading({ from: 0, text: "\r" }, now)).toEqual({ from: 1, to: 1, insert: "\n\n## 09:05 ", caret: 12 });
  expect(insertTimeHeading({ from: 4, text: "メモ" }, now)).toEqual({ from: 6, to: 6, insert: "\n\n## 09:05 ", caret: 17 });
});

test("「- 」の行の Enter は次の行も「- 」で始め、中身のない「- 」の行では「- 」を消す", () => {
  expect(continueList({ from: 0, text: "- 買う" }, 4)).toEqual({ from: 4, to: 4, insert: "\n- ", caret: 7 });
  // 字下げはそろえる。行の途中なら、そこで分ける
  expect(continueList({ from: 10, text: "  - ab" }, 5)).toEqual({ from: 15, to: 15, insert: "\n  - ", caret: 20 });
  expect(continueList({ from: 3, text: "- " }, 2)).toEqual({ from: 3, to: 5, insert: "", caret: 3 });
  // 字下げは残す
  expect(continueList({ from: 3, text: "  - " }, 4)).toEqual({ from: 5, to: 7, insert: "", caret: 5 });
  // CR の付いた「- 」は中身のない行とみなさず、CR を消さない
  expect(continueList({ from: 0, text: "- \r" }, 2)).toEqual({ from: 2, to: 2, insert: "\n- ", caret: 5 });
  // カーソルが CR の前なら CR は次の行へ送られ、その行の Enter でリストを終えられる
  expect(continueList({ from: 0, text: "- \r" }, 2)).toEqual({ from: 2, to: 2, insert: "\n- ", caret: 5 });
  // 字下げと中身の空の判定は、コアと同じ空白の文字（全角の空白・U+0085 は空白、U+FEFF は空白でない）
  expect(continueList({ from: 0, text: "\u3000- " }, 3)).toEqual({ from: 1, to: 3, insert: "", caret: 1 });
  expect(continueList({ from: 0, text: "- \u0085" }, 3)).toEqual({ from: 0, to: 3, insert: "", caret: 0 });
  expect(continueList({ from: 0, text: "\t- a" }, 4)).toEqual({ from: 4, to: 4, insert: "\n\t- ", caret: 8 });
  expect(continueList({ from: 0, text: "- \uFEFF" }, 3)).toEqual({ from: 3, to: 3, insert: "\n- ", caret: 6 });
  expect(continueList({ from: 0, text: "\uFEFF- a" }, 4)).toBeNull();
  // CR は字下げに数えない（次の行へ写さない）
  expect(continueList({ from: 0, text: "\r- a" }, 4)).toBeNull();
  // 「- 」より前のカーソルと、リストでない行はふつうの Enter
  expect(continueList({ from: 0, text: "- 買う" }, 1)).toBeNull();
  expect(continueList({ from: 0, text: "-買う" }, 3)).toBeNull();
  expect(continueList({ from: 0, text: "メモ" }, 2)).toBeNull();
});

test("空の行の判定はコアと同じ空白の文字で、CR を含む行は空とみなさない", () => {
  for (const yes of ["", "  ", "\t", "\u3000", "\u0085", "\u00A0 "]) expect(blank(yes), JSON.stringify(yes)).toBe(true);
  for (const no of ["\r", " \r ", "\uFEFF", "a", "- "]) expect(blank(no), JSON.stringify(no)).toBe(false);
});
