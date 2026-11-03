import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { DocSaver, SaveStatus } from "./autosave.svelte";
import { EDITING_MS } from "./board.svelte";
import { EditorPresence, InputTracker, UiReporter } from "./ui_report.svelte";

beforeEach(() => {
  vi.useFakeTimers({ now: 1_800_000_000_000 });
});

afterEach(() => {
  vi.useRealTimers();
});

const DATE = "2026-11-03";

function presence(saved: string, editor: string) {
  let text = editor;
  const p = new EditorPresence();
  p.saver = new DocSaver(new SaveStatus(), "reflection", DATE, { body: saved, rev: 4 }, {
    text: () => text,
    replace: (t) => (text = t),
  });
  p.text = () => text;
  return { p, set: (t: string) => (text = t) };
}

test("守る行は、未保存の変更がある行と、入力中のカーソル行を、最後に保存した本文の行番号にしたもの", () => {
  const { p } = presence("a\nb\nc", "new\na\nB\nc");
  // フォーカスが無ければ、未保存の変更がある行だけ
  expect(p.editors(Date.now(), true)).toEqual([{ doc: "reflection", date: DATE, rev: 4, lines: [2] }]);
  p.focused = true;
  p.lastInputAt = Date.now();
  p.cursorLines = [4];
  expect(p.editors(Date.now(), true)[0].lines).toEqual([2, 3]);
  // アプリが前面に無いか、最後の入力から EDITING_MS 経ったら、カーソル行は含めない
  expect(p.editors(Date.now(), false)[0].lines).toEqual([2]);
  expect(p.editors(Date.now() + EDITING_MS, true)[0].lines).toEqual([2]);
  // 変換中は時間にかかわらず含める
  p.composing = true;
  expect(p.editors(Date.now() + EDITING_MS, true)[0].lines).toEqual([2, 3]);
  // 足しただけの行にカーソルがあっても、守る行は無い
  p.cursorLines = [1];
  expect(p.editors(Date.now(), true)[0].lines).toEqual([2]);
});

test("保存済みの本文のままで、入力中でなければ、守る行を入れない", () => {
  const { p } = presence("a\nb", "a\nb");
  p.focused = true;
  p.cursorLines = [1];
  expect(p.editors(Date.now(), true)).toEqual([]);
});

test("中身が変わったときだけ送り、送れなかったら次に送り直す", async () => {
  let state = { editingItems: [], editors: [] };
  const send = vi.fn().mockRejectedValueOnce(new Error("not ready")).mockResolvedValue(null);
  const r = new UiReporter(() => state, send);
  r.update();
  await vi.advanceTimersByTimeAsync(0);
  r.update();
  r.update();
  expect(send).toHaveBeenCalledTimes(2);
  state = { editingItems: [{ itemId: "x", field: "text" }] as never, editors: [] };
  r.update();
  expect(send).toHaveBeenCalledTimes(3);
});

test("項目・振り返り・ノートの分を1つにまとめて送り、入力中でなくなる時刻に知らせ直す", async () => {
  const sent: unknown[] = [];
  const save = new SaveStatus();
  const tracker = new InputTracker(save, async (s) => sent.push(s), true);
  const ref = presence("a\nb", "a\nB");
  const note = presence("x\ny", "x\ny");
  Object.assign(tracker.reflection, { saver: ref.p.saver, text: ref.p.text });
  const noteSaver = new DocSaver(save, "note", DATE, { body: "x\ny", rev: 7 }, { text: () => "x\ny", replace: () => {} });
  Object.assign(tracker.note, { saver: noteSaver, text: note.p.text, focused: true, lastInputAt: Date.now(), cursorLines: [2] });
  tracker.items = { editingItems: () => [{ itemId: "i", field: "text" }] as never, editingUntil: () => null };
  tracker.activity();
  expect(sent.at(-1)).toEqual({
    editingItems: [{ itemId: "i", field: "text" }],
    editors: [
      { doc: "reflection", date: DATE, rev: 4, lines: [2] },
      { doc: "note", date: DATE, rev: 7, lines: [2] },
    ],
  });
  // ノートの最後の入力から EDITING_MS 経つと、ノートのカーソル行を外して知らせ直す
  await vi.advanceTimersByTimeAsync(EDITING_MS + 1);
  expect((sent.at(-1) as { editors: unknown[] }).editors).toEqual([{ doc: "reflection", date: DATE, rev: 4, lines: [2] }]);
  // アプリが前面から外れたら、入力中の項目の分も（focus で決まるものは）求め直す
  tracker.setActive(false);
  expect(tracker.active).toBe(false);
  tracker.stop();
  const n = sent.length;
  tracker.activity();
  expect(sent.length).toBe(n);
});
