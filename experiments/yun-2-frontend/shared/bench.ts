// 計測用コード。Svelte 版・Solid 版で同じファイルを使う。
// 画面の DOM（data-testid の付いた要素）だけを操作するので、フレームワークに依存しない。
//
// 1. signalReady()：画面の部品を組み上げてレイアウトが済んだら `mounted`、
//    最初の描画まで進んだら `ready` を Rust 側に知らせる。Rust 側はプロセス起動からの経過時間を
//    標準出力に出す（scripts/measure.py が読む）。
//    見えない特別ワークスペースに開いたウィンドウでは描画が起きないので `ready` は届かない。
//    そのときは `mounted` を起動完了の目安にする。
// 2. 環境変数 YUN2_MODE=bench で起動したときは、続けて入力反応のベンチマークを走らせ、
//    結果を `report` コマンドで Rust 側に渡す（描画を待たないので、見えないウィンドウでも動く）。
import { invoke } from '@tauri-apps/api/core';
import { createImeGuard } from './ime-guard';
import { runListCheck } from './listcheck';

const nextFrame = () => new Promise<void>((r) => requestAnimationFrame(() => r()));
const nextTask = () => new Promise<void>((r) => setTimeout(r, 0));

/** 2回 requestAnimationFrame を待つ＝最初のフレームが描画まで進んだとみなす */
async function afterPaint() {
  await nextFrame();
  await nextFrame();
}

export async function signalReady() {
  // フレームワークの更新を流し切ってからレイアウトを確定させる
  await nextTask();
  forceLayout();
  const mountedMs = performance.now(); // WebView のナビゲーション開始からの時間
  const mode = await invoke<string>('mounted', { frontMs: mountedMs, visibility: document.visibilityState });
  afterPaint().then(() => invoke('ready', { frontMs: performance.now() }));
  if (mode === 'bench') {
    // 起動直後の揺れを避けて少し待つ
    await new Promise((r) => setTimeout(r, 500));
    const result = await runBench();
    await invoke('report', { json: JSON.stringify(result) });
  } else if (mode === 'listcheck') {
    // 「その場で編集する項目の一覧」の自動チェック（listcheck.ts）
    await new Promise((r) => setTimeout(r, 500));
    const result = await runListCheck().catch((e) => ({ error: String(e && e.stack ? e.stack : e) }));
    await invoke('report', { json: JSON.stringify(result) });
  }
}

const $ = <T extends Element>(sel: string) => document.querySelector<T>(sel)!;
const count = () => document.querySelectorAll('[data-testid=todo-item]').length;

/** 条件が満たされるまで待つ（DOM の変化を MutationObserver で監視） */
function waitFor(cond: () => boolean): Promise<void> {
  if (cond()) return Promise.resolve();
  return new Promise((resolve, reject) => {
    const mo = new MutationObserver(() => {
      if (cond()) {
        mo.disconnect();
        clearTimeout(timer);
        resolve();
      }
    });
    mo.observe(document.body, { subtree: true, childList: true, attributes: true, characterData: true });
    const timer = setTimeout(() => {
      mo.disconnect();
      reject(new Error('waitFor timeout'));
    }, 2000);
  });
}

/** 強制的にスタイル計算とレイアウトを行わせる（描画直前まで含めた時間にするため） */
const forceLayout = () => document.body.getBoundingClientRect().height;

function typeAndEnter(text: string) {
  const input = $<HTMLInputElement>('[data-testid=todo-input]');
  input.value = text;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true, cancelable: true }));
}

async function addOne(i: number) {
  const before = count();
  typeAndEnter(`項目 ${i} の内容`);
  await waitFor(() => count() === before + 1);
  forceLayout();
}

async function measureAdds(n: number, startIndex: number) {
  const t0 = performance.now();
  for (let i = 0; i < n; i++) await addOne(startIndex + i);
  return (performance.now() - t0) / n;
}

async function measureToggles(n: number) {
  const toggles = document.querySelectorAll<HTMLInputElement>('[data-testid=todo-toggle]');
  // 決まった順番（擬似乱数）で選ぶ
  let seed = 12345;
  const rand = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
  const t0 = performance.now();
  for (let i = 0; i < n; i++) {
    const idx = Math.floor(rand() * toggles.length);
    const item = document.querySelectorAll('[data-testid=todo-item]')[idx];
    const before = item.getAttribute('data-done');
    document.querySelectorAll<HTMLInputElement>('[data-testid=todo-toggle]')[idx].click();
    await waitFor(() => item.getAttribute('data-done') !== before);
    forceLayout();
  }
  return (performance.now() - t0) / n;
}

async function measureClear() {
  const t0 = performance.now();
  $<HTMLButtonElement>('[data-testid=clear-all]').click();
  await waitFor(() => count() === 0);
  forceLayout();
  return performance.now() - t0;
}

async function measureEditor(n: number) {
  const view = (window as unknown as { __cmView: import('@codemirror/view').EditorView }).__cmView;
  const t0 = performance.now();
  for (let i = 0; i < n; i++) {
    const pos = view.state.doc.length;
    view.dispatch({ changes: { from: pos, insert: i % 40 === 39 ? '\n' : 'あ' }, selection: { anchor: pos + 1 } });
    forceLayout();
  }
  return (performance.now() - t0) / n;
}

/**
 * IME 判定の「論理」だけを合成イベントで確かめる。本物の IME の挙動ではない
 * （本物はオーナーの実機確認：RESULTS.md の確認手順）。
 */
async function checkImeLogic() {
  const input = $<HTMLInputElement>('[data-testid=todo-input]');
  const results: Record<string, boolean> = {};
  const fireKey = (init: KeyboardEventInit, keyCode?: number) => {
    const ev = new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true, cancelable: true, ...init });
    if (keyCode !== undefined) Object.defineProperty(ev, 'keyCode', { get: () => keyCode });
    input.dispatchEvent(ev);
  };
  const settle = async () => {
    await Promise.resolve();
    await nextTask();
  };
  const setDraft = (v: string) => {
    input.value = v;
    input.dispatchEvent(new Event('input', { bubbles: true }));
  };

  // (a) 変換中（isComposing=true）の Enter では追加しない
  let before = count();
  setDraft('変換中a');
  input.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
  fireKey({ isComposing: true });
  await settle();
  results.composingEnterIgnored = count() === before;

  // (b) keyCode 229 の Enter では追加しない
  fireKey({}, 229);
  await settle();
  results.keyCode229Ignored = count() === before;

  // (c) compositionend の直後（同じタスク内）の最初の Enter は追加しない
  input.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: '変換中a' }));
  fireKey({});
  await settle();
  results.enterRightAfterCompositionEndIgnored = count() === before;

  // (d) その後の普通の Enter では追加する
  await new Promise((r) => setTimeout(r, 10));
  fireKey({});
  await settle();
  results.laterEnterAdds = count() === before + 1;

  // (e) compositionend の後、次のタスクに入ってからの Enter は追加する（印が残り続けない）
  before = count();
  setDraft('変換後b');
  input.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: '変換後b' }));
  await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => setTimeout(r, 0));
  fireKey({});
  await settle();
  results.enterInLaterTaskAdds = count() === before + 1;

  // 判定関数そのものも同じ規則か（両版で同じ ime-guard.ts を使っていることの確認）
  const g = createImeGuard();
  results.guardModuleSane =
    g.ignoreReason(new KeyboardEvent('keydown', { key: 'Enter', isComposing: true })) === 'isComposing' &&
    g.ignoreReason(new KeyboardEvent('keydown', { key: 'Enter' })) === null;
  return results;
}

async function runBench() {
  (window as unknown as { __yun2NoLog: boolean }).__yun2NoLog = true;
  const ime = await checkImeLogic();
  await measureClear();

  const addMsPerOp: number[] = [];
  for (let round = 0; round < 10; round++) addMsPerOp.push(await measureAdds(100, round * 100));
  const toggleMsPerOp = await measureToggles(200);
  const clearMs = await measureClear();
  const editorMsPerOp = await measureEditor(500);

  return {
    userAgent: navigator.userAgent,
    visibility: document.visibilityState,
    ime,
    // 1件追加あたりの時間（入力→Enter→DOM 反映→レイアウトまで）。round i は一覧が i*100 件ある状態から 100 件追加
    addMsPerOp,
    // 1000 件ある状態で、完了の切り替え1回あたりの時間
    toggleMsPerOp,
    // 1000 件を一度に消す時間
    clearMs,
    // CodeMirror に1文字ずつ挿入する1回あたりの時間（両版で同じエディタ。参考値）
    editorMsPerOp,
  };
}
