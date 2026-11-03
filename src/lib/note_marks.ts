// ノートの送信済みの印（画面設計書 4章、デザインガイド 6章「ノートの送信済み行」）と、行送りで送る行。
// 見た目のうち、行に付ける印の文字（「→ TODO」「→ BLOCKER」）と属性だけを持つ（色と位置は NoteView）。
//
// - 印を付ける行は、コアがエディタの本文の各行を送った記録と照合して決める（`note_marks`。データモデル
//   設計書 3章 note_send）。画面は求めた結果を `setMarks` で入れ、次に求め直すまでの入力では、印を行の
//   動きに合わせて動かす（本文を変えた行の印が外れるのは、求め直したとき）。
// - 同じ行に印が2つ付くこともある（同じ本文の行を2回送った後に1行を消したときなど）。そのときは送り先の
//   種類ごとに1つずつ出す。

import { RangeSetBuilder, StateEffect, StateField, type EditorState } from "@codemirror/state";
import { Decoration, EditorView, type DecorationSet } from "@codemirror/view";
import type { Id, ItemKind, NoteMark } from "./bindings";

/** 印の文字（日英で共通の英字。デザインガイド 7章）。 */
export const SENT_LABEL: Record<ItemKind, string> = { todo: "→ TODO", blocker: "→ BLOCKER" };

/** 1つの印：行の始まりの位置と、送り先。 */
interface Mark {
  pos: number;
  kind: ItemKind;
  itemId: Id;
}

export interface SentMarks {
  /** 行の順。 */
  marks: readonly Mark[];
  deco: DecorationSet;
}

/** 印を入れ直す（`marks` は、いまのエディタの本文で数えた行。コアの `note_marks` の結果）。 */
export const setMarks = StateEffect.define<readonly NoteMark[]>();

const lineDeco = new Map<string, Decoration>();

/** 行の印の飾り（送り先の組ごとに1つ作って使い回す）。 */
function decoOf(kinds: readonly ItemKind[]): Decoration {
  const label = (["todo", "blocker"] as const)
    .filter((k) => kinds.includes(k))
    .map((k) => SENT_LABEL[k])
    .join("  ");
  let d = lineDeco.get(label);
  if (d === undefined) {
    d = Decoration.line({ attributes: { "data-sent": label } });
    lineDeco.set(label, d);
  }
  return d;
}

function build(state: EditorState, marks: Mark[]): SentMarks {
  // 行の始まりにそろえ、行の順に並べる（取り込みや削除で、印が行の途中に来ても）
  const at = marks
    .map((m) => ({ ...m, pos: state.doc.lineAt(Math.min(m.pos, state.doc.length)).from }))
    .sort((a, b) => a.pos - b.pos);
  const b = new RangeSetBuilder<Decoration>();
  for (let i = 0; i < at.length; ) {
    let j = i;
    const kinds: ItemKind[] = [];
    while (j < at.length && at[j].pos === at[i].pos) kinds.push(at[j++].kind);
    b.add(at[i].pos, at[i].pos, decoOf(kinds));
    i = j;
  }
  return { marks: at, deco: b.finish() };
}

/** 送信済みの印。 */
export const marksField = StateField.define<SentMarks>({
  create: () => ({ marks: [], deco: Decoration.none }),
  update(value, tr) {
    let next = value;
    if (tr.docChanged && value.marks.length > 0) {
      // 行の始まりに入れた文字の後ろへ動かす（行の始まりで Enter を押したら、印は下へ動いた本文に付いて
      // いく。文字を入れただけなら、行の途中になった位置を build がその行の始まりにそろえる）
      next = build(
        tr.state,
        value.marks.map((m) => ({ ...m, pos: tr.changes.mapPos(m.pos, 1) })),
      );
    }
    for (const e of tr.effects) {
      if (!e.is(setMarks)) continue;
      const doc = tr.state.doc;
      next = build(
        tr.state,
        e.value
          .filter((m) => m.line >= 1 && m.line <= doc.lines)
          .map((m) => ({ pos: doc.line(m.line).from, kind: m.kind, itemId: m.itemId })),
      );
    }
    return next;
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.deco),
});

/** Sent に出す、送り先ごとの印のある行の数（同じ行に同じ送り先の印が2つあっても1行）。 */
export function sentCounts(v: SentMarks): Record<ItemKind, number> {
  const lines: Record<ItemKind, Set<number>> = { todo: new Set(), blocker: new Set() };
  for (const m of v.marks) lines[m.kind].add(m.pos);
  return { todo: lines.todo.size, blocker: lines.blocker.size };
}

/** 項目 `itemId` へ送った印のある行（1始まり）。無ければ `null`。 */
export function markedLine(state: EditorState, itemId: Id): number | null {
  const m = state.field(marksField, false)?.marks.find((x) => x.itemId === itemId);
  return m === undefined ? null : state.doc.lineAt(m.pos).number;
}

/**
 * 行送りで送る行（1始まり。両端を含む）：主な選択範囲のある行。選択範囲が次の行の先頭で終わるとき
 * （行を選んで Shift+↓ で下へ広げたときなど）は、その行を含めない。
 */
export function sendRange(state: EditorState): { from: number; to: number } {
  const sel = state.selection.main;
  const doc = state.doc;
  const from = doc.lineAt(sel.from).number;
  let to = doc.lineAt(sel.to).number;
  if (!sel.empty && to > from && doc.line(to).from === sel.to) to--;
  return { from, to };
}
