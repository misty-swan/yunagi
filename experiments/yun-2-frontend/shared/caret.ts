// contenteditable の中のカーソル操作。prototype/app.js の setCaretIn・caretOffset・onFirstLine・onLastLine と同じ考え方。
// Svelte 版・Solid 版で同じファイルを使う。

function textNodes(el: Node): Text[] {
  const w = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  const out: Text[] = [];
  let t: Node | null;
  while ((t = w.nextNode())) out.push(t as Text);
  return out;
}

/** el の中の、先頭から off 文字目にカーソルを置く */
export function setCaretIn(el: HTMLElement, off: number, end = off) {
  const ts = textNodes(el);
  const point = (n: number): [Node, number] => {
    let rem = Math.max(0, n);
    for (const t of ts) {
      if (rem <= t.length) return [t, rem];
      rem -= t.length;
    }
    return ts.length ? [ts[ts.length - 1], ts[ts.length - 1].length] : [el, 0];
  };
  const r = document.createRange();
  r.setStart(...point(off));
  r.setEnd(...point(end));
  const s = getSelection()!;
  s.removeAllRanges();
  s.addRange(r);
}

/** カーソル（選択範囲の始点と終点）が el の先頭から何文字目か */
export function caretRange(el: HTMLElement): [number, number] {
  const s = getSelection();
  if (!s || !s.rangeCount) return [0, 0];
  const r = s.getRangeAt(0);
  if (!el.contains(r.startContainer)) return [el.textContent!.length, el.textContent!.length];
  const pre = document.createRange();
  pre.selectNodeContents(el);
  pre.setEnd(r.startContainer, r.startOffset);
  const start = pre.toString().length;
  pre.setEnd(r.endContainer, r.endOffset);
  return [start, pre.toString().length];
}

export const caretOffset = (el: HTMLElement) => caretRange(el)[0];

function caretRect(): DOMRect | null {
  const s = getSelection();
  if (!s || !s.rangeCount) return null;
  const r = s.getRangeAt(0).cloneRange();
  r.collapse(true);
  const rs = r.getClientRects();
  return rs.length ? rs[0] : null;
}

/** 折り返した項目で、カーソルが先頭の行にあるか */
export function onFirstLine(el: HTMLElement) {
  const ls = el.getClientRects();
  if (ls.length <= 1) return true;
  const c = caretRect();
  return !c || c.top < ls[0].bottom - 1;
}

/** 折り返した項目で、カーソルが末尾の行にあるか */
export function onLastLine(el: HTMLElement) {
  const ls = el.getClientRects();
  if (ls.length <= 1) return true;
  const c = caretRect();
  return !c || c.bottom > ls[ls.length - 1].top + 1;
}

const itemText = (id: string) =>
  document.querySelector<HTMLElement>(`[data-item-id="${CSS.escape(id)}"] [data-testid=item-text]`);

/** 項目 id の本文にフォーカスし、offset 文字目にカーソルを置く（'end' は末尾） */
export function focusItem(id: string, offset: number | 'end') {
  const t = itemText(id);
  if (!t) return;
  t.focus({ preventScroll: true });
  setCaretIn(t, offset === 'end' ? t.textContent!.length : offset);
  t.scrollIntoView({ block: 'nearest' });
}

/** いまフォーカスのある項目の id（なければ null） */
export function focusedItemId(): string | null {
  const a = document.activeElement;
  const li = a && a.closest && a.closest<HTMLElement>('[data-item-id]');
  return li ? li.dataset.itemId! : null;
}
