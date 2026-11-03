import { expect, test } from "vitest";
import { continueList, insertTimeHeading, timeHeadingOf, timeHeadingPrefix } from "./note";

test("時刻付き見出しの判定はコアと同じ形（## の後に H:MM か HH:MM、その後は行末か空白）", () => {
  expect(timeHeadingOf("## 10:00 定例")).toEqual({ time: "10:00", title: "定例", timeFrom: 3, timeTo: 8 });
  expect(timeHeadingOf("## 9:30")).toEqual({ time: "9:30", title: "", timeFrom: 3, timeTo: 7 });
  expect(timeHeadingOf("## 23:59　振り返り")?.title).toBe("振り返り");
  expect(timeHeadingOf("## 10:00 ")?.title).toBe("");
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
  expect(insertTimeHeading({ from: 4, text: "メモ" }, now)).toEqual({ from: 6, to: 6, insert: "\n\n## 09:05 ", caret: 17 });
});

test("「- 」の行の Enter は次の行も「- 」で始め、中身のない「- 」の行では「- 」を消す", () => {
  expect(continueList({ from: 0, text: "- 買う" }, 4)).toEqual({ from: 4, to: 4, insert: "\n- ", caret: 7 });
  // 字下げはそろえる。行の途中なら、そこで分ける
  expect(continueList({ from: 10, text: "  - ab" }, 5)).toEqual({ from: 15, to: 15, insert: "\n  - ", caret: 20 });
  expect(continueList({ from: 3, text: "- " }, 2)).toEqual({ from: 3, to: 5, insert: "", caret: 3 });
  // 字下げは残す
  expect(continueList({ from: 3, text: "  - " }, 4)).toEqual({ from: 5, to: 7, insert: "", caret: 5 });
  // 「- 」より前のカーソルと、リストでない行はふつうの Enter
  expect(continueList({ from: 0, text: "- 買う" }, 1)).toBeNull();
  expect(continueList({ from: 0, text: "-買う" }, 3)).toBeNull();
  expect(continueList({ from: 0, text: "メモ" }, 2)).toBeNull();
});
