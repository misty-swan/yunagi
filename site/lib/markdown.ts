// Markdown（利用者ガイド・変更履歴・サイト用の本文）を HTML にする。変換は marked（依存はこれだけ）。
// 取り込む文書は自分たちのリポジトリのものだが、文書の書き方の誤りがそのままサイトに出ないよう、
// 生の HTML・画像・サイト外へ出ない相対リンクは、例外にして止める（ビルドが落ちる）。
import { Marked, type Tokens } from "marked";
import { esc } from "./html.ts";

export interface Heading {
  depth: number;
  text: string;
  id: string;
}

export interface Rendered {
  html: string;
  headings: Heading[];
}

export interface RenderOptions {
  /** 見出しの深さをずらす（変更履歴を、別の文書の h2 の下に置くとき） */
  shift?: number;
  /** 本文の先頭の h1 を捨てる（ずらして置くとき、見出しは呼ぶ側が書く） */
  dropFirstH1?: boolean;
  /** `<!-- slot:名前 -->` の1行を、この HTML に置き換える（生成する表など） */
  slots?: Record<string, string>;
  /** 横にはみ出す表を包む領域の名前（キーボードで動かす領域には名前が要る） */
  tableLabel: string;
}

/** 見出しの ID（GitHub の見出しの ID に近い形）。日本語の見出しはそのまま残す。 */
export function slugify(text: string): string {
  return text
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, "")
    .replace(/\s+/g, "-");
}

const SLOT = /^<!-- slot:([a-z][a-z0-9-]*) -->$/;

/** 生成物の置き場所（スロット）で本文を分ける。 */
function splitSlots(src: string): Array<{ md: string } | { slot: string }> {
  const parts: Array<{ md: string } | { slot: string }> = [];
  let buf: string[] = [];
  for (const line of src.split("\n")) {
    const m = SLOT.exec(line);
    if (m) {
      parts.push({ md: buf.join("\n") });
      parts.push({ slot: m[1] });
      buf = [];
    } else {
      buf.push(line);
    }
  }
  parts.push({ md: buf.join("\n") });
  return parts;
}

/**
 * HTML のコメント（記載ルールなど、読み手に見せないもの）を取り除く。
 * コードの囲み（``` と ~~~）の中と、行内コード（`...`）の中は触らない。閉じていないコメントは、
 * 以降の本文を黙って捨ててしまうので、例外にして止める。
 */
function stripComments(md: string): string {
  const out: string[] = [];
  let fence: string | null = null; // 開いている囲みの記号（``` か ~~~）
  let inComment = false;
  for (const line of md.split("\n")) {
    if (!inComment) {
      const m = /^ {0,3}(`{3,}|~{3,})/.exec(line);
      if (fence === null && m) fence = m[1][0].repeat(3);
      else if (fence !== null && m && m[1][0] === fence[0]) {
        out.push(line);
        fence = null;
        continue;
      }
    }
    if (fence !== null) {
      out.push(line);
      continue;
    }
    // 行内コードの区間は、そのまま残す。残りの区間でコメントを探す
    let result = "";
    for (const part of line.split(/(`+[^`]*`+)/)) {
      if (!inComment && part.startsWith("`")) {
        result += part;
        continue;
      }
      let rest = part;
      while (rest !== "") {
        if (inComment) {
          const end = rest.indexOf("-->");
          if (end < 0) {
            rest = "";
          } else {
            inComment = false;
            rest = rest.slice(end + 3);
          }
        } else {
          const start = rest.indexOf("<!--");
          if (start < 0) {
            result += rest;
            rest = "";
          } else {
            result += rest.slice(0, start);
            inComment = true;
            rest = rest.slice(start + 4);
          }
        }
      }
    }
    out.push(result);
  }
  if (inComment) throw new Error("閉じていない HTML コメント（<!-- に対応する --> が無い）");
  return out.join("\n");
}

export function renderMarkdown(src: string, options: RenderOptions): Rendered {
  const shift = options.shift ?? 0;
  const headings: Heading[] = [];
  const used = new Map<string, number>();
  let skipH1 = options.dropFirstH1 ?? false;

  const marked = new Marked({
    gfm: true,
    renderer: {
      heading(this, { tokens, depth }: Tokens.Heading): string {
        const inner = this.parser.parseInline(tokens);
        if (depth === 1 && skipH1) {
          skipH1 = false;
          return "";
        }
        const level = Math.min(depth + shift, 6);
        const text = tokens.map((t) => ("text" in t ? String(t.text) : "")).join("");
        let id = slugify(text) || "section";
        const n = used.get(id) ?? 0;
        used.set(id, n + 1);
        if (n > 0) id = `${id}-${n + 1}`;
        headings.push({ depth: level, text, id });
        return `<h${level} id="${esc(id)}">${inner}</h${level}>\n`;
      },
      html({ text }: Tokens.HTML | Tokens.Tag): string {
        // 生の HTML は通さず、書いたとおりの文字として出す
        return esc(text);
      },
      image({ href }: Tokens.Image): string {
        throw new Error(`画像は使えない（外部の読み込みを入れない決まり）: ${href}`);
      },
      link(this, { href, title, tokens }: Tokens.Link): string {
        const inner = this.parser.parseInline(tokens);
        if (!/^(https:\/\/|mailto:|#|\/(?!\/))/.test(href)) {
          throw new Error(`リンクの行き先が決まりに合わない（https:// か mailto: か # か / で始める）: ${href}`);
        }
        const t = title ? ` title="${esc(title)}"` : "";
        return `<a href="${esc(href)}"${t}>${inner}</a>`;
      },
    },
  });

  const out: string[] = [];
  for (const part of splitSlots(src)) {
    if ("slot" in part) {
      const html = options.slots?.[part.slot];
      if (html === undefined) throw new Error(`スロット ${part.slot} に入れる内容が無い`);
      out.push(html);
      continue;
    }
    const html = marked.parse(stripComments(part.md), { async: false });
    out.push(html);
  }
  const html = out
    .join("\n")
    .replaceAll("<table>", `<div class="scroll" role="region" tabindex="0" aria-label="${esc(options.tableLabel)}"><table>`)
    .replaceAll("</table>", "</table></div>")
    .replaceAll("<pre>", '<pre tabindex="0">');
  return { html, headings };
}
