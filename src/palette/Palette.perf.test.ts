// 検索パレットで1文字ごとにかかる処理の時間を測る（テスト計画書 5章「入力応答」16ms 以内）。手で走らせる：
//   YUNAGI_PERF=1 pnpm vitest run src/palette/Palette.perf.test.ts
// 結果の上限（50 件）まで、10KB ほどのノートの結果を出した状態で打つ。打つたびに行うのは、入力欄の更新と、
// 日付とコマンドの照合と、一覧の描き直し（結果の切り出しは答えが届いたときだけ）。答えが届いたときの
// 切り出しと描画も測る。jsdom の描画を含むので、実際の WebView より遅めに出る。

import { flushSync, mount, unmount } from "svelte";
import { expect, test } from "vitest";
import type { SearchHit } from "../lib/bindings";
import { Palette as PaletteState, SEARCH_LIMIT } from "../lib/palette.svelte";
import { prefs } from "../lib/prefs.svelte";
import Palette from "./Palette.svelte";

function report(label: string, times: number[]): number {
  times.sort((a, b) => a - b);
  const p = (q: number) => times[Math.ceil(times.length * q) - 1];
  console.log(`${label}: median=${p(0.5).toFixed(2)}ms p95=${p(0.95).toFixed(2)}ms max=${p(1).toFixed(2)}ms`);
  return p(0.95);
}

/** 10KB ほどのノート（日本語。一致する語はおよそ中ほどに1つ）。 */
function note(i: number): string {
  const lines: string[] = [];
  for (let k = 0; k < 120; k++) lines.push(k === 60 ? `- レート制限の件 ${i}` : `- 議題 ${i}-${k} について、API の応答を確かめる`);
  return lines.join("\n");
}

// Vitest は環境変数を import.meta.env にも入れる
test.skipIf(!import.meta.env.YUNAGI_PERF)("1文字ごとの処理と、答えが届いたときの描画の時間", async () => {
  prefs.lang = "ja";
  const hits: SearchHit[] = Array.from({ length: SEARCH_LIMIT }, (_, i) => ({
    kind: "note",
    refId: `2026-01-${String((i % 28) + 1).padStart(2, "0")}`,
    date: `2026-01-${String((i % 28) + 1).padStart(2, "0")}`,
    body: note(i),
    resolutionNote: null,
    carriedTo: null,
  }));
  let answer: (h: SearchHit[]) => void = () => {};
  const palette = new PaletteState(() => new Promise((r) => (answer = r)), 0);
  const app = mount(Palette, {
    target: document.body,
    props: {
      palette,
      today: "2026-11-03",
      context: { mode: "board", sidebarOpen: true, layout: "A", carryPending: true, saveFailing: false, updateAvailable: false, mac: false },
      onRun: () => {},
    },
  });
  palette.show();
  flushSync();

  // 答えが届いたとき（切り出しと描画）
  const arrive: number[] = [];
  for (let i = 0; i < 20; i++) {
    palette.input(`レート制限${i % 2 === 0 ? "" : "の"}`);
    await new Promise((r) => setTimeout(r, 0));
    const t = performance.now();
    answer(hits);
    await Promise.resolve();
    await Promise.resolve();
    flushSync();
    arrive.push(performance.now() - t);
  }
  expect(document.querySelectorAll("#pal-list li.hit")).toHaveLength(SEARCH_LIMIT);

  // 1文字ごと（答えは返さず、出している結果のまま打ち続ける）
  const input = document.querySelector<HTMLInputElement>("[data-palette] input")!;
  const keys: number[] = [];
  let q = "レート制限";
  for (let i = 0; i < 200; i++) {
    q = i % 20 === 19 ? "レート" : `${q}あ`;
    const t = performance.now();
    input.value = q;
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    flushSync();
    keys.push(performance.now() - t);
  }
  report("答えが届いたとき", arrive);
  const p95 = report("1文字ごと", keys);
  expect(p95).toBeLessThan(16);
  unmount(app);
});
