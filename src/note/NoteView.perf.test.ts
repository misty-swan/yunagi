// 長いノートで1文字ごとにかかる処理の時間を測る（テスト計画書 5章「入力応答」16ms 以内）。手で走らせる：
//   YUNAGI_PERF=1 pnpm vitest run src/note/NoteView.perf.test.ts
// 見出しの飾りと Outline・送信済みの印（CodeMirror の更新）と、入力中の状態の知らせ（InputTracker が
// 本文を写して、最後に保存した本文との差分から守る行を求める）を、1文字ごとに行う。jsdom の描画を含む
// ので、実際の WebView より遅めに出る。

import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { flushSync, mount, unmount } from "svelte";
import { expect, test, vi } from "vitest";
import { SaveStatus } from "../lib/autosave.svelte";
import { FontLoader } from "../lib/fonts.svelte";
import { Notice } from "../lib/notice.svelte";
import { prefs } from "../lib/prefs.svelte";
import { InputTracker } from "../lib/ui_report.svelte";
import NoteView from "./NoteView.svelte";

Range.prototype.getClientRects ??= () => [] as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => new DOMRect();

/** 見積もりの上限に近い長さ（ノート 10KB の4倍ほど。40 の時刻付き見出し、840 行）。 */
function longNote(): string {
  const lines: string[] = [];
  for (let h = 0; h < 40; h++) {
    lines.push(`## ${String(8 + (h % 12)).padStart(2, "0")}:00 打ち合わせ ${h}`);
    for (let k = 0; k < 20; k++) lines.push(`- 議題 ${h}-${k} について、API の応答を確かめる`);
  }
  return lines.join("\n");
}

/** 飾りも知らせも無い CodeMirror（jsdom の描画にかかる時間の目安）。 */
function measureBare(body: string) {
  const v = new EditorView({ parent: document.body, state: EditorState.create({ doc: body, extensions: [EditorView.lineWrapping] }) });
  const times: number[] = [];
  for (let i = 0; i < 300; i++) {
    const line = v.state.doc.line(2 + ((i * 37) % (v.state.doc.lines - 2)));
    const t = performance.now();
    v.dispatch({ changes: { from: line.to, insert: "あ" }, selection: { anchor: line.to + 1 }, userEvent: "input.type" });
    times.push(performance.now() - t);
  }
  v.destroy();
  report("飾りの無い CodeMirror", times);
}

function report(label: string, times: number[]) {
  times.sort((a, b) => a - b);
  const p = (q: number) => times[Math.ceil(times.length * q) - 1].toFixed(2);
  console.log(`${label}: median=${p(0.5)}ms p95=${p(0.95)}ms max=${p(1)}ms`);
}

async function measure(label: string, body: string, saveFails: boolean) {
  let rev = 1;
  mockIPC((cmd, args) => {
    const a = args as Record<string, unknown>;
    if (cmd === "get_text_doc") return { body: a.doc === "note" ? body : "", rev: a.doc === "note" ? 1 : 0 };
    if (cmd === "save_text_doc") {
      // 保存に失敗し続けると、最後に保存した本文との差分が大きくなる
      if (saveFails) throw { kind: "storage", detail: "x" };
      return { result: "saved", rev: ++rev, updated: true };
    }
    if (cmd === "note_marks") return [];
    return null;
  });
  prefs.noteFont = "body";
  const save = new SaveStatus();
  const tracker = new InputTracker(save, undefined, true);
  let view: EditorView | null = null;
  const app = mount(NoteView, {
    target: document.body,
    props: {
      date: "2026-11-03",
      save,
      presence: tracker.note,
      rev: 1,
      onActivity: tracker.activity,
      visible: true,
      narrow: false,
      mac: false,
      notice: new Notice(),
      items: "",
      jump: null,
      onJumped: () => {},
      font: new FontLoader(async () => true),
      get view() {
        return view;
      },
      set view(v) {
        view = v;
      },
    },
  });
  await vi.waitFor(() => {
    flushSync();
    expect(tracker.note.saver).not.toBeNull();
  });
  // 時間を測る performance.now は進め方を変えない
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date"] });
  const v = view!;
  v.contentDOM.focus();
  const times: number[] = [];
  // 1秒に10文字の速さで、本文の6か所（上から下まで散らばった行）を50文字ずつ打つ。保存に失敗し
  // 続けると、まだ保存していない変更が本文全体に散らばる
  for (const n of [50, 200, 350, 500, 650, 800]) {
    for (let i = 0; i < 50; i++) {
      const line = v.state.doc.line(n);
      const t = performance.now();
      v.dispatch({ changes: { from: line.to, insert: "あ" }, selection: { anchor: line.to + 1 }, userEvent: "input.type" });
      flushSync();
      times.push(performance.now() - t);
      await vi.advanceTimersByTimeAsync(100);
    }
  }
  report(label, times);
  vi.useRealTimers();
  unmount(app);
  tracker.stop();
  document.body.innerHTML = "";
  clearMocks();
}

// Vitest は環境変数を import.meta.env にも入れる
test.skipIf(!import.meta.env.YUNAGI_PERF)("長いノートでの1文字ごとの処理の時間", async () => {
  const body = longNote();
  console.log(`note: ${body.length} chars, ${body.split("\n").length} lines`);
  measureBare(body);
  await measure("保存できているとき", body, false);
  await measure("保存に失敗し続けているとき", body, true);
}, 120_000);
