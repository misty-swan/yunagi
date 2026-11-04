// ふりかえり（S-08）に出す、振り返りとノートの本文の取り出し（画面設計書 9章）。
//
// - Daily notes の GOOD / STUCK：振り返りのテンプレートの見出し（画面設計書 3章「各領域の表示」。
//   日本語・英語の両方）の下の、最初の文字のある行。見出しの文字は `i18n.ts` の `reflectionTemplate`
//   から読む（テンプレートを入れるときと食い違わないため）。
// - 見出しは「## 」で始まる行（振り返りのエディタと同じ。3章）。

import { MESSAGES } from "./i18n";

const HEADING_PREFIX = "## ";

/** テンプレートの `n` 番目の見出しの文字（日英とも。小文字にそろえる）。 */
function templateHeadings(n: number): Set<string> {
  const names = Object.values(MESSAGES).map((m) =>
    m.reflectionTemplate
      .filter((line) => line.startsWith(HEADING_PREFIX))
      [n].slice(HEADING_PREFIX.length)
      .trim()
      .toLowerCase(),
  );
  return new Set(names);
}

const GOOD_HEADINGS = templateHeadings(0);
const STUCK_HEADINGS = templateHeadings(1);

/** 行頭の箇条書きの印（`- ` `* `）を除く。印だけの行は空になる。 */
export function stripBullet(line: string): string {
  return line.replace(/^\s*[-*](?:[ \t]+|$)/, "").trim();
}

/** 見出し `headings` の下の最初の文字のある行（次の見出しまで）。無ければ `null`。 */
function firstLineUnder(reflection: string, headings: Set<string>): string | null {
  let inside = false;
  for (const line of reflection.split("\n")) {
    if (line.startsWith(HEADING_PREFIX)) {
      if (inside) return null;
      inside = headings.has(line.slice(HEADING_PREFIX.length).trim().toLowerCase());
      continue;
    }
    if (!inside) continue;
    const text = stripBullet(line);
    if (text !== "") return text;
  }
  return null;
}

/** 振り返りの「良かったこと」「詰まったこと」の1行目（無ければ `null`）。 */
export function reflectionPicks(reflection: string): { good: string | null; stuck: string | null } {
  return {
    good: firstLineUnder(reflection, GOOD_HEADINGS),
    stuck: firstLineUnder(reflection, STUCK_HEADINGS),
  };
}

/** プレビューに出す振り返りの1行。 */
export type ReflectionBlock = { kind: "heading" | "line"; text: string };

/** 振り返りの全文を、見出しと行に分ける。空の行と、印だけの箇条書きの行は出さない。 */
export function reflectionBlocks(reflection: string): ReflectionBlock[] {
  const blocks: ReflectionBlock[] = [];
  for (const line of reflection.split("\n")) {
    if (line.startsWith(HEADING_PREFIX)) {
      const text = line.slice(HEADING_PREFIX.length).trim();
      if (text !== "") blocks.push({ kind: "heading", text });
      continue;
    }
    const text = stripBullet(line);
    if (text !== "") blocks.push({ kind: "line", text });
  }
  return blocks;
}

/** 時刻の文字（`9:30`。ノートに書いたままの形ではなく、時と分から作る）。 */
export function timeLabel(hour: number, minute: number): string {
  return `${hour}:${String(minute).padStart(2, "0")}`;
}
