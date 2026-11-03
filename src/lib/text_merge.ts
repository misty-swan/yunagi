// ノート・振り返りの行の差分と取り込み（docs/design/01-architecture.md 4.1「ノート・振り返りへの
// 取り込み方」「ノート・振り返りの守る行」）。
//
// - diffLines：もとの本文から別の本文への、行の変更（ひとかたまりずつ）。
// - mergeText：最後に保存した本文（もと）をもとに、エージェント側の変更（もと→コアの最新）と、
//   利用者のまだ保存していない変更（もと→エディタ）を合わせる。
// - unsavedLines：もとの本文の行のうち、エディタで書き換えたか削除して、まだ保存していない行
//   （守る行の「未保存の変更がある行」。行番号はもとの本文で数える）。
//
// 行は "\n" で分ける（コアの数え方と同じ。空の本文は空の1行、末尾の改行の後ろも空の1行）。

/** もとの行 `[start, end)`（0 始まり）を `lines` に置き換える変更。`start === end` なら行の追加。 */
export interface Hunk {
  start: number;
  end: number;
  lines: string[];
}

/** 本文の行。 */
export function splitLines(body: string): string[] {
  return body.split("\n");
}

/** 差分を細かく求める行の数の上限（もとと相手の、共通の先頭・末尾を除いた行の数の積）。超えたら、残りを1つの置き換えとして扱う（取り込みの結果は正しいまま、まとまりが大きくなるだけ）。 */
const DIFF_CELLS_LIMIT = 4_000_000;

/**
 * `a` と `b` の最長共通部分列の行の組（a の位置, b の位置。前から順）を、Myers の O((N+M)D) の方法で
 * 求める（D は足した行と消した行の数）。途中の状態を覚える量が DIFF_CELLS_LIMIT を超えるほど変えた行が
 * 多ければ、求めずに `null`。
 */
function myersPairs(a: string[], b: string[]): [number, number][] | null {
  const n = a.length;
  const m = b.length;
  const max = n + m;
  const off = max + 1;
  const maxD = Math.min(max, Math.floor(DIFF_CELLS_LIMIT / (2 * max + 3)));
  let v = new Int32Array(2 * max + 3);
  // trace[d]：d 回目の手を指す前の、対角線 k ごとの最も進んだ x
  const trace: Int32Array[] = [];
  for (let d = 0; d <= maxD; d++) {
    trace.push(v);
    v = v.slice();
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? v[off + k + 1] : v[off + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && a[x] === b[y]) {
        x++;
        y++;
      }
      v[off + k] = x;
      if (x >= n && y >= m) return backtrack(trace, off, n, m);
    }
  }
  return null;
}

/** `myersPairs` の手を終わりからたどって、一致した行の組を求める。 */
function backtrack(trace: Int32Array[], off: number, n: number, m: number): [number, number][] {
  const pairs: [number, number][] = [];
  let x = n;
  let y = m;
  for (let d = trace.length - 1; d >= 0; d--) {
    const v = trace[d];
    const k = x - y;
    const prevK = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? k + 1 : k - 1;
    const prevX = d === 0 ? 0 : v[off + prevK];
    const prevY = d === 0 ? 0 : prevX - prevK;
    while (x > prevX && y > prevY) {
      pairs.push([x - 1, y - 1]);
      x--;
      y--;
    }
    x = prevX;
    y = prevY;
  }
  return pairs.reverse();
}

/** `base` から `other` への行の変更。重ならず、もとの行の順に並ぶ。隣り合う変更は1つにまとめる。 */
export function diffLines(base: string[], other: string[]): Hunk[] {
  let head = 0;
  while (head < base.length && head < other.length && base[head] === other[head]) head++;
  let tail = 0;
  while (
    tail < base.length - head &&
    tail < other.length - head &&
    base[base.length - 1 - tail] === other[other.length - 1 - tail]
  )
    tail++;
  const a = base.slice(head, base.length - tail);
  const b = other.slice(head, other.length - tail);
  if (a.length === 0 && b.length === 0) return [];
  // 一致する行の組（a の位置, b の位置）を、最長共通部分列で求める。変えた行が少なければ Myers の方法で
  // （変えた行の数に比例する時間で）求め、多ければ表を埋めて求める
  const myers = myersPairs(a, b);
  const pairs: [number, number][] = myers ?? [];
  if (myers === null && a.length > 0 && b.length > 0 && a.length * b.length <= DIFF_CELLS_LIMIT) {
    const w = b.length + 1;
    const lcs = new Uint32Array((a.length + 1) * w);
    for (let i = a.length - 1; i >= 0; i--) {
      for (let j = b.length - 1; j >= 0; j--) {
        lcs[i * w + j] =
          a[i] === b[j]
            ? lcs[(i + 1) * w + j + 1] + 1
            : Math.max(lcs[(i + 1) * w + j], lcs[i * w + j + 1]);
      }
    }
    let i = 0;
    let j = 0;
    while (i < a.length && j < b.length) {
      if (a[i] === b[j]) {
        pairs.push([i, j]);
        i++;
        j++;
      } else if (lcs[(i + 1) * w + j] >= lcs[i * w + j + 1]) i++;
      else j++;
    }
  }
  const hunks: Hunk[] = [];
  let i = 0;
  let j = 0;
  for (const [pi, pj] of [...pairs, [a.length, b.length] as [number, number]]) {
    if (pi > i || pj > j) {
      hunks.push({ start: head + i, end: head + pi, lines: b.slice(j, pj) });
    }
    i = pi + 1;
    j = pj + 1;
  }
  return hunks;
}

type Side = "mine" | "theirs";

interface SidedHunk extends Hunk {
  side: Side;
}

const isInsert = (h: Hunk) => h.start === h.end;

/** 2つの変更が、同じ行にかかわるか（取り込みで1つのまとまりとして扱うか）。 */
function touches(x: Hunk, y: Hunk): boolean {
  if (isInsert(x) && isInsert(y)) return x.start === y.start;
  // 追加は、すぐ上の行（start - 1）に付いているものとして扱う
  if (isInsert(x)) return y.start < x.start && x.start <= y.end;
  if (isInsert(y)) return x.start < y.start && y.start <= x.end;
  return x.start < y.end && y.start < x.end;
}

/** もとの行 `[from, to)` に、`hunks`（その中の変更）を当てた行。 */
function applyHunks(base: string[], from: number, to: number, hunks: Hunk[]): string[] {
  const out: string[] = [];
  let pos = from;
  for (const h of [...hunks].sort((x, y) => x.start - y.start)) {
    out.push(...base.slice(pos, h.start), ...h.lines);
    pos = h.end;
  }
  out.push(...base.slice(pos, to));
  return out;
}

/**
 * もと `base` に、エージェント側の変更（`base` → `theirs`）と利用者のまだ保存していない変更
 * （`base` → `mine`）を合わせた本文（docs/design/01-architecture.md 4.1「取り込み方」）。
 *
 * - 重ならない変更は、両方を残す。
 * - 同じ位置への行の追加は、両方を残す（利用者の行が先）。
 * - 利用者の新しい行は、すぐ上の保存済みの行に付いているものとして扱う。その行がエージェントの
 *   変更（書き換え・削除・移動）の範囲に入ったら、範囲を書き換えた場合は新しい行の後ろ、削除・移動
 *   した場合は範囲のすぐ上の行の後ろに置く（移動は「元の位置から削除して別の位置に追加」として扱う）。
 *   エージェントの新しい行が、利用者の書き換えた範囲に付いているときも同じく、範囲の後ろに置く。
 * - 同じ行を両方が変えたら、その位置に、利用者の結果の行、続けてエージェントの結果の行を置く。
 *   両方の変更の範囲が一部だけ重なるときは、重なりをつないだ範囲を1つのまとまりとし、そのまとまりの
 *   利用者の結果とエージェントの結果を続けて置く（どちらを残すかは利用者が画面で決める）。
 */
export function mergeText(base: string, theirs: string, mine: string): string {
  if (theirs === base) return mine;
  if (mine === base || mine === theirs) return theirs;
  const b = splitLines(base);
  const all: SidedHunk[] = [
    ...diffLines(b, splitLines(mine)).map((h) => ({ ...h, side: "mine" as const })),
    ...diffLines(b, splitLines(theirs)).map((h) => ({ ...h, side: "theirs" as const })),
  ];
  // かかわり合う変更をまとめる
  const group = all.map((_, i) => i);
  const root = (i: number): number => (group[i] === i ? i : (group[i] = root(group[i])));
  for (let i = 0; i < all.length; i++) {
    for (let j = i + 1; j < all.length; j++) {
      if (touches(all[i], all[j])) group[root(i)] = root(j);
    }
  }
  const groups = new Map<number, SidedHunk[]>();
  all.forEach((h, i) => {
    const r = root(i);
    groups.set(r, [...(groups.get(r) ?? []), h]);
  });
  const regions = [...groups.values()].map((hunks) => {
    const ranges = hunks.filter((h) => !isInsert(h));
    const start = ranges.length > 0 ? Math.min(...ranges.map((h) => h.start)) : hunks[0].start;
    const end = ranges.length > 0 ? Math.max(...ranges.map((h) => h.end)) : hunks[0].start;
    return { start, end, lines: regionLines(b, start, end, hunks) };
  });
  regions.sort((x, y) => x.start - y.start || x.end - y.end);
  return applyHunks(b, 0, b.length, regions).join("\n");
}

/** まとまり（もとの行 `[start, end)` と、その中の変更）を合わせた行。 */
function regionLines(base: string[], start: number, end: number, hunks: SidedHunk[]): string[] {
  const mine = hunks.filter((h) => h.side === "mine");
  const theirs = hunks.filter((h) => h.side === "theirs");
  if (theirs.length === 0) return applyHunks(base, start, end, mine);
  if (mine.length === 0) return applyHunks(base, start, end, theirs);
  const inserted = (hs: SidedHunk[]) => hs.flatMap((h) => h.lines);
  const onlyInserts = (hs: SidedHunk[]) => hs.every(isInsert);
  if (onlyInserts(mine) && onlyInserts(theirs)) return [...inserted(mine), ...inserted(theirs)];
  // 片方が追加だけなら、もう片方の範囲の結果の後ろに置く
  if (onlyInserts(mine)) return [...applyHunks(base, start, end, theirs), ...inserted(mine)];
  if (onlyInserts(theirs)) return [...applyHunks(base, start, end, mine), ...inserted(theirs)];
  const mineLines = applyHunks(base, start, end, mine);
  const theirLines = applyHunks(base, start, end, theirs);
  // 両方が同じ結果にしたなら、1つだけ残す（二重にしない）
  if (mineLines.length === theirLines.length && mineLines.every((l, k) => l === theirLines[k])) {
    return mineLines;
  }
  return [...mineLines, ...theirLines];
}

/**
 * もと `base` の行のうち、`editor` で書き換えたか削除した行の番号（1 始まり、昇順。もとの本文で
 * 数える）。足しただけの行は含めない（docs/design/01-architecture.md 4.1「未保存の変更がある行」）。
 */
export function unsavedLines(
  base: string,
  editor: string,
  hunks: readonly Hunk[] = diffLines(splitLines(base), splitLines(editor)),
): number[] {
  return hunks.flatMap((h) =>
    Array.from({ length: h.end - h.start }, (_, k) => h.start + k + 1),
  );
}

/**
 * エディタ `editor` の行番号 `lines`（1 始まり）を、もと `base` の行番号に置き換える（守る行の
 * 「カーソル行」を、最後に保存した本文で数えて知らせるため。docs/design/01-architecture.md 4.1）。
 * 変えていない行はもとの同じ行に、書き換えた行は書き換える前の行（まとまりの全部）にする。足した
 * だけの行は、エージェントが指定できないので含めない。結果は昇順で重ならない。
 */
export function baseLinesOf(
  base: string,
  editor: string,
  lines: readonly number[],
  hunks: readonly Hunk[] = diffLines(splitLines(base), splitLines(editor)),
): number[] {
  const out = new Set<number>();
  for (const line of lines) {
    // エディタの 0 始まりの行 i を、変更の前から順にたどってもとの行にする
    const i = line - 1;
    let shift = 0;
    let mapped: number[] | null = null;
    for (const h of hunks) {
      const from = h.start + shift;
      if (i < from) break;
      if (i < from + h.lines.length) {
        mapped = Array.from({ length: h.end - h.start }, (_, k) => h.start + k);
        break;
      }
      shift += h.lines.length - (h.end - h.start);
    }
    for (const b of mapped ?? [i - shift]) out.add(b + 1);
  }
  return [...out].sort((a, b) => a - b);
}
