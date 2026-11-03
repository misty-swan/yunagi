// 検索結果の、一致した部分の切り出し（画面設計書 6章「本文の一部」、データモデル設計書 4.1）。
//
// コアは本文の全体を返すので、一致した位置は画面で探し直す。大文字・小文字の扱いはコアにそろえる：
// 3文字以上の検索語は、ASCII 以外も含めて区別しない（trigram の既定）。2文字以下は ASCII の英字だけ
// 区別しない（SQLite の LIKE）。文字数は Unicode の文字（コードポイント）で数える。
// 位置は JavaScript の文字列の位置（UTF-16）で返す。

/** 3文字以上の検索語は、ASCII 以外の大文字・小文字もそろえる（コアの索引で探す長さ）。 */
const MIN_INDEXED_CHARS = 3;

/** 一致した範囲（`from` から `to` の手前まで）。 */
export interface MatchRange {
  from: number;
  to: number;
}

/** 長さを変えずに大文字・小文字をそろえる（そろえると長さの変わる文字はそのまま）。 */
function fold(s: string, unicode: boolean): string {
  if (!unicode) return s.replace(/[A-Z]+/g, (c) => c.toLowerCase());
  // ふつうは全体をまとめてそろえても長さが変わらない（10KB のノートを 50 件切り出すので、1文字ずつは遅い）
  const whole = s.toLowerCase();
  if (whole.length === s.length) return whole;
  let out = "";
  for (const ch of s) {
    const lower = ch.toLowerCase();
    out += lower.length === ch.length ? lower : ch;
  }
  return out;
}

/**
 * 検索語（前後の空白を除いたもの）。除く空白はコアの `str::trim`（Unicode の White_Space）にそろえる。
 * JavaScript の `trim` は U+FEFF も除き、U+0085 は除かないので、そのままでは画面とコアで検索語がずれる。
 */
export function searchTerm(query: string): string {
  return query.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, "");
}

/** `text` の中で、最初に検索語 `query` と一致した範囲。無ければ `null`。 */
export function findMatch(text: string, query: string): MatchRange | null {
  const term = searchTerm(query);
  if (term === "") return null;
  const unicode = [...term].length >= MIN_INDEXED_CHARS;
  const at = fold(text, unicode).indexOf(fold(term, unicode));
  return at < 0 ? null : { from: at, to: at + term.length };
}

/** 結果の行に出す本文の一部。`mark` は `text` の中の一致した範囲（無ければ `null`）。 */
export interface Snippet {
  text: string;
  mark: MatchRange | null;
  /** 前・後ろを切ったか（「…」を添える）。 */
  cutStart: boolean;
  cutEnd: boolean;
}

/** 一致した所の前に残す文字数（UTF-16）。 */
const BEFORE = 24;
/** 切り出す長さの上限（UTF-16。行の幅に収まらない分は表示で切る）。 */
const MAX_LEN = 160;

/** サロゲートペアの途中なら、1つ前へずらす。 */
function safeStart(text: string, i: number): number {
  const c = text.charCodeAt(i);
  return i > 0 && c >= 0xdc00 && c <= 0xdfff ? i - 1 : i;
}

function safeEnd(text: string, i: number): number {
  const c = text.charCodeAt(i - 1);
  return i < text.length && c >= 0xd800 && c <= 0xdbff ? i + 1 : i;
}

/**
 * 本文 `body` から、検索語 `query` と一致した所を含む1行を切り出す（長い行は一致した所の少し前から）。
 * 一致しなければ（コアと画面の一致の決まりがずれた場合など）、空白でない最初の行を出す。
 */
export function snippetOf(body: string, query: string): Snippet {
  const m = findMatch(body, query);
  const at = m?.from ?? Math.max(0, body.search(/\S/));
  const lineStart = body.lastIndexOf("\n", at - 1) + 1;
  const nl = body.indexOf("\n", at);
  const lineEnd = nl < 0 ? body.length : nl;
  const start = m === null ? lineStart : safeStart(body, Math.max(lineStart, m.from - BEFORE));
  const end = safeEnd(body, Math.min(lineEnd, start + MAX_LEN));
  const text = body.slice(start, end).replace(/\s+$/, "");
  const mark =
    m === null || m.from >= start + text.length
      ? null
      : { from: m.from - start, to: Math.min(m.to, start + text.length) - start };
  return { text, mark, cutStart: start > lineStart, cutEnd: end < lineEnd };
}
