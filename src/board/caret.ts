// 1行の入力欄（contenteditable の span）の、カーソルの位置の読み書き。

/** `el` の中のカーソルの文字位置。`el` の中に無ければ `null`。 */
export function caretOffset(el: HTMLElement): number | null {
  const sel = el.ownerDocument.getSelection();
  if (sel === null || sel.rangeCount === 0) return null;
  const r = sel.getRangeAt(0);
  if (!el.contains(r.startContainer)) return null;
  const pre = el.ownerDocument.createRange();
  pre.selectNodeContents(el);
  pre.setEnd(r.startContainer, r.startOffset);
  return pre.toString().length;
}

/** 選択が空（範囲を選んでいない）か。 */
export function caretCollapsed(el: HTMLElement): boolean {
  const sel = el.ownerDocument.getSelection();
  return sel === null || sel.rangeCount === 0 || sel.getRangeAt(0).collapsed;
}

/** カーソルを `el` の文字位置 `offset`（`end` なら末尾。本文より後ろなら末尾）に置く。 */
export function setCaret(el: HTMLElement, offset: number | "end") {
  const doc = el.ownerDocument;
  const sel = doc.getSelection();
  if (sel === null) return;
  const len = el.textContent?.length ?? 0;
  let rest = offset === "end" ? len : Math.min(offset, len);
  const range = doc.createRange();
  const walker = doc.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n !== null; n = walker.nextNode()) {
    const t = n.textContent?.length ?? 0;
    if (rest <= t) {
      range.setStart(n, rest);
      range.collapse(true);
      sel.removeAllRanges();
      sel.addRange(range);
      return;
    }
    rest -= t;
  }
  range.selectNodeContents(el);
  range.collapse(false);
  sel.removeAllRanges();
  sel.addRange(range);
}

/**
 * カーソルが、折り返した `el` の最初（`first`）か最後（`last`）の行にあるか（↑ / ↓ で隣の項目へ移る
 * 判定。操作仕様書 2章）。行の位置が分からないときは、先頭・末尾にあるかで決める。
 */
export function caretOnEdgeLine(el: HTMLElement, edge: "first" | "last"): boolean {
  const offset = caretOffset(el);
  const len = el.textContent?.length ?? 0;
  if (offset === null) return true;
  const lines = el.getClientRects();
  const sel = el.ownerDocument.getSelection();
  const rect = sel && sel.rangeCount > 0 ? sel.getRangeAt(0).getClientRects()[0] : undefined;
  if (lines.length <= 1) return true;
  if (rect === undefined) return edge === "first" ? offset === 0 : offset === len;
  const line = edge === "first" ? lines[0] : lines[lines.length - 1];
  const mid = rect.top + rect.height / 2;
  return mid >= line.top && mid <= line.bottom;
}
