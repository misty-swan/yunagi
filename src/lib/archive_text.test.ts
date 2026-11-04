import { expect, test } from "vitest";
import { reflectionBlocks, reflectionPicks, stripBullet, timeLabel } from "./archive_text";

test("GOOD / STUCK は、テンプレートの見出し（日本語・英語）の下の最初の文字のある行", () => {
  const ja = "## 良かったこと\n- 設計を終えた\n- 二つ目\n\n## 詰まったこと\n- \n\n- CI が落ちる\n## 明日の一手\n- 続き";
  expect(reflectionPicks(ja)).toEqual({ good: "設計を終えた", stuck: "CI が落ちる" });
  const en = "## Went well\n* shipped\n## Got stuck\nflaky tests\n";
  expect(reflectionPicks(en)).toEqual({ good: "shipped", stuck: "flaky tests" });
  // 画面の言語によらず、両方の見出しを読む。大文字小文字と前後の空白は問わない
  expect(reflectionPicks("## went WELL  \n- ok").good).toBe("ok");
});

test("見出しの下に文字が無いか、見出しが無ければ null。次の見出しまでを探す", () => {
  expect(reflectionPicks("")).toEqual({ good: null, stuck: null });
  expect(reflectionPicks("## 良かったこと\n- \n## 詰まったこと\n- 詰まり")).toEqual({ good: null, stuck: "詰まり" });
  // 見出しより前の行や、ほかの見出しの下の行は取らない
  expect(reflectionPicks("前置き\n## 明日の一手\n- 続き\n## 良かったこと\n")).toEqual({ good: null, stuck: null });
  // 「## 」で始まらない行は見出しではない
  expect(reflectionPicks("#良かったこと\n- x")).toEqual({ good: null, stuck: null });
});

test("プレビューの振り返りは、見出しと行に分け、空の行・印だけの行を出さない", () => {
  expect(reflectionBlocks("## 良かったこと\n- 進んだ\n\n- \n## 詰まったこと\n  * 待ち\n##  \n本文")).toEqual([
    { kind: "heading", text: "良かったこと" },
    { kind: "line", text: "進んだ" },
    { kind: "heading", text: "詰まったこと" },
    { kind: "line", text: "待ち" },
    { kind: "line", text: "本文" },
  ]);
  expect(stripBullet("  - 項目")).toBe("項目");
  expect(stripBullet("-")).toBe("");
  expect(timeLabel(9, 5)).toBe("9:05");
});
