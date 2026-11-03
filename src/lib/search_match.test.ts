import { expect, test } from "vitest";
import { findMatch, searchTerm, snippetOf } from "./search_match";

test("検索語の前後の空白を除き、全体を1つの文字列として探す", () => {
  expect(findMatch("API の応答を確かめる", "  応答を ")).toEqual({ from: 5, to: 8 });
  expect(findMatch("API の応答", "応答 確かめ")).toBeNull();
  expect(findMatch("何か", "   ")).toBeNull();
  // 除く空白はコアの str::trim（Unicode の White_Space）にそろえる
  expect(searchTerm("\u0085 　語\t")).toBe("語");
  expect(searchTerm("﻿語")).toBe("﻿語");
});

test("大文字・小文字の扱いはコアにそろえる（3文字以上は ASCII 以外も、2文字以下は ASCII だけそろえる）", () => {
  expect(findMatch("Rate Limit", "rate")).toEqual({ from: 0, to: 4 });
  expect(findMatch("Rate Limit", "ra")).toEqual({ from: 0, to: 2 });
  expect(findMatch("ÄBC über", "äbc")).toEqual({ from: 0, to: 3 });
  expect(findMatch("Äb", "äb")).toBeNull();
  // 全角英字も、3文字以上ならそろえる
  expect(findMatch("ＡＰＩの件", "ａｐｉ")).toEqual({ from: 0, to: 3 });
});

test("本文の一部：一致した所のある1行を、長ければ少し前から切り出す", () => {
  const body = "## 10:00 定例\n- レート制限を調べる\n- 権限待ち";
  expect(snippetOf(body, "制限")).toEqual({
    text: "- レート制限を調べる",
    mark: { from: 5, to: 7 },
    cutStart: false,
    cutEnd: false,
  });
  const long = `${"あ".repeat(100)}一致${"い".repeat(300)}`;
  const s = snippetOf(long, "一致");
  expect(s.cutStart).toBe(true);
  expect(s.cutEnd).toBe(true);
  expect(s.text.slice(s.mark!.from, s.mark!.to)).toBe("一致");
  expect(s.mark!.from).toBe(24);
});

test("一致しなければ空白でない最初の行を出す。サロゲートペアの途中では切らない", () => {
  expect(snippetOf("\n\n  最初の行\n次", "無い")).toEqual({ text: "  最初の行", mark: null, cutStart: false, cutEnd: false });
  const emoji = `${"😀".repeat(40)}語`;
  const s = snippetOf(emoji, "語");
  expect(s.text.startsWith("😀")).toBe(true);
  expect(s.text.slice(s.mark!.from, s.mark!.to)).toBe("語");
});
