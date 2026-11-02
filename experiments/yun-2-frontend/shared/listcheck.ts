// 「その場で編集する項目の一覧」の自動チェック。Svelte 版・Solid 版で同じファイルを使う。
// 環境変数 YUN2_MODE=listcheck で起動したときに走り、結果を `report` コマンドで Rust 側に渡す。
// 合成イベント（プログラムから作ったキー入力）なので、本物の IME の挙動は確かめられない。
import { caretRange, setCaretIn } from './caret';
import { core, newItemId } from './core';

const spans = () => [...document.querySelectorAll<HTMLElement>('[data-testid=item-text]')];
const rows = () => [...document.querySelectorAll<HTMLElement>('[data-item-id]')];
const texts = () => spans().map((s) => s.textContent);
const $ = <T extends Element>(sel: string) => document.querySelector<T>(sel)!;
const forceLayout = () => document.body.getBoundingClientRect().height;
const nextTask = () => new Promise<void>((r) => setTimeout(r, 0));
const settle = async () => {
  await Promise.resolve();
  await nextTask();
};

function key(el: HTMLElement, k: string, init: KeyboardEventInit = {}, keyCode?: number) {
  const ev = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init });
  if (keyCode !== undefined) Object.defineProperty(ev, 'keyCode', { get: () => keyCode });
  el.dispatchEvent(ev);
  return ev;
}

function focusAt(el: HTMLElement, start: number, end = start) {
  el.focus();
  setCaretIn(el, start, end);
}

/** いまのフォーカスが spans()[index] にあり、選択範囲が [start, end] か */
function focusIs(index: number, start: number, end = start) {
  const el = spans()[index];
  const [s, e] = caretRange(el);
  return document.activeElement === el && s === start && e === end;
}

const reloadCount = () => $('[data-testid=reload-count]').textContent;

/** 読み込み直しボタンを押し、画面に反映されるまで待つ（回数の表示が変わるのを待つ） */
function clickReload(): Promise<number> {
  const before = reloadCount();
  return new Promise((resolve, reject) => {
    const mo = new MutationObserver(() => {
      if (reloadCount() !== before) {
        mo.disconnect();
        forceLayout();
        resolve(performance.now() - t0);
      }
    });
    mo.observe(document.body, { subtree: true, childList: true, characterData: true });
    setTimeout(() => {
      mo.disconnect();
      reject(new Error('reload timeout'));
    }, 3000);
    const t0 = performance.now();
    $<HTMLButtonElement>('[data-testid=reload]').click();
  });
}

/** 読み込み直しの前後で、行の要素・本文のテキストノード・フォーカス・選択範囲が保たれたか */
async function reloadKeeps(index: number, start: number, end: number, during?: () => void) {
  const el = spans()[index];
  focusAt(el, start, end);
  const rowsBefore = rows();
  const textNodesBefore = spans().map((s) => s.firstChild);
  const textsBefore = texts();
  during?.();
  await clickReload();
  await settle();
  const rowsAfter = rows();
  const textNodesAfter = spans().map((s) => s.firstChild);
  const changed = texts()
    .map((t, i) => (t !== textsBefore[i] ? i : -1))
    .filter((i) => i >= 0);
  return {
    focusAndSelectionKept: focusIs(index, start, end),
    sameFocusedElement: document.activeElement === el,
    rowsRecreated: rowsAfter.filter((r, i) => r !== rowsBefore[i]).length,
    focusedTextNodeKept: textNodesAfter[index] === textNodesBefore[index],
    otherTextNodesReplaced: textNodesAfter.filter((n, i) => i !== index && n !== textNodesBefore[i]).length,
    changedItems: changed,
    focusedTextUnchanged: texts()[index] === textsBefore[index],
  };
}

async function editingChecks() {
  const r: Record<string, boolean> = {};
  const counts: Record<string, number> = {};
  const expect = (name: string, ok: boolean) => (r[name] = ok);
  const base = texts().slice();

  // 1. 行の途中の Enter：2件に分け、後ろの項目の先頭へ
  focusAt(spans()[0], 2);
  key(spans()[0], 'Enter');
  await settle();
  expect('splitMiddle', texts()[0] === base[0]!.slice(0, 2) && texts()[1] === base[0]!.slice(2) && focusIs(1, 0));

  // 2. 行頭の Enter：直上に空の項目を足し、そこへ
  key(spans()[1], 'Enter');
  await settle();
  expect('enterAtStartAddsAbove', texts()[1] === '' && texts()[2] === base[0]!.slice(2) && focusIs(1, 0));

  // 3. 空の項目の Backspace：削除して直前の項目の末尾へ
  key(spans()[1], 'Backspace');
  await settle();
  expect('backspaceOnEmptyDeletes', texts()[1] === base[0]!.slice(2) && focusIs(0, 2));

  // 4. 行末の Enter：直下に空の項目
  key(spans()[0], 'Enter');
  await settle();
  expect('enterAtEndAddsBelow', texts()[1] === '' && texts()[2] === base[0]!.slice(2) && focusIs(1, 0));
  key(spans()[1], 'Backspace');
  await settle();

  // 5. 空でない項目の Backspace は普通の文字削除（項目は消さない）
  const n = spans().length;
  key(spans()[0], 'Backspace');
  await settle();
  expect('backspaceOnNonEmptyKeepsItem', spans().length === n);

  // 6. ↓・↑：隣の項目へ。カーソルの位置（何文字目か）を保つ
  focusAt(spans()[2], 1);
  key(spans()[2], 'ArrowDown');
  await settle();
  const downOk = focusIs(3, 1);
  key(spans()[3], 'ArrowUp');
  await settle();
  expect('arrowDownUp', downOk && focusIs(2, 1));

  // 7. 文字の入力がデータ（コア）に届く（execCommand は本物と同じ input イベントを起こす）
  focusAt(spans()[2], spans()[2].textContent!.length);
  document.execCommand('insertText', false, 'X');
  await settle();
  const saved = core.load().map((i) => i.text);
  expect('typingReachesCore', texts()[2]!.endsWith('X') && saved[2] === texts()[2]);

  // 7b. 1文字入力したとき、フレームワークが DOM を書き戻していないか（DOM の変更記録の数を数える。
  //     ブラウザ自身の入力で 1 件。それより多ければ、フレームワークが同じ値などを書き込んでいる。
  //     変換中にこれが起きると IME が壊れる恐れがある）。カーソルとテキストノードも保たれるか
  {
    const t = spans()[2];
    const len = t.textContent!.length;
    focusAt(t, len);
    const node = t.firstChild;
    let records = 0;
    const mo = new MutationObserver((rs) => (records += rs.length));
    mo.observe(t, { subtree: true, characterData: true, childList: true });
    document.execCommand('insertText', false, 'Y');
    await settle();
    counts.typingDomMutations = records + mo.takeRecords().length;
    mo.disconnect();
    expect('typingKeepsCaretAndTextNode', t.firstChild === node && focusIs(2, len + 1));
  }

  // 7c. 空の項目（行末の Enter で作ったもの）に入力しても、画面とデータが一致する
  focusAt(spans()[2], spans()[2].textContent!.length);
  key(spans()[2], 'Enter');
  await settle();
  {
    const t = spans()[3];
    let records = 0;
    const mo = new MutationObserver((rs) => (records += rs.length));
    mo.observe(t, { subtree: true, characterData: true, childList: true });
    document.execCommand('insertText', false, 'あい');
    await settle();
    counts.typingIntoEmptyDomMutations = records + mo.takeRecords().length;
    mo.disconnect();
    expect('typingIntoEmptyItemOk', t.textContent === 'あい' && core.load()[3].text === 'あい' && focusIs(3, 2));
  }

  // 8. IME：変換中の Enter では分割しない。compositionend 直後の Enter も分割せず、改行も入れない
  const el = spans()[2];
  focusAt(el, 1);
  const count = spans().length;
  el.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
  key(el, 'Enter', { isComposing: true });
  await settle();
  const composingOk = spans().length === count;
  key(el, 'Enter', {}, 229);
  await settle();
  const k229Ok = spans().length === count;
  el.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: 'x' }));
  const ev = key(el, 'Enter');
  await settle();
  expect('imeComposingEnterNoSplit', composingOk);
  expect('imeKeyCode229NoSplit', k229Ok);
  expect('imeEnterRightAfterCompositionEndNoSplit', spans().length === count && ev.defaultPrevented);
  await nextTask();
  key(el, 'Enter');
  await settle();
  expect('imeLaterEnterSplits', spans().length === count + 1);
  return { checks: r, counts };
}

async function reloadChecks() {
  const agent = $<HTMLInputElement>('[data-testid=reload-agent]');
  if (agent.checked) agent.click();
  // a. 中身が変わらない読み込み直し：選択範囲（2〜4文字目）を置いて
  const plain = await reloadKeeps(1, 1, 3);
  // b. 変換を始めた状態（compositionstart だけ送る）での読み込み直し
  const el = spans()[1];
  const duringComposition = await reloadKeeps(1, 2, 2, () =>
    el.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true })),
  );
  el.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: '' }));
  await nextTask();
  // c. エージェントが別の項目を変えた読み込み直し
  agent.click();
  await settle();
  const withAgentChange = await reloadKeeps(1, 2, 2);
  agent.click();
  await settle();
  // d. 自動の読み込み直し（3秒ごと）をオンにして 3.5 秒待つ
  focusAt(spans()[1], 1, 2);
  const before = reloadCount();
  const auto = $<HTMLInputElement>('[data-testid=reload-auto]');
  auto.click();
  await new Promise((r) => setTimeout(r, 3500));
  auto.click();
  const autoReload = { reloaded: reloadCount() !== before, focusAndSelectionKept: focusIs(1, 1, 2) };
  return { plain, duringComposition, withAgentChange, autoReload };
}

const median = (xs: number[]) => {
  const s = [...xs].sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
};

async function perf() {
  // 1000 件にする（コアに入れて読み込み直す）
  const many = core.load();
  while (many.length < 1000) many.push({ id: newItemId(), text: `項目 ${many.length} の内容` });
  core.save(many);
  const fill = await clickReload();
  await settle();
  // 中身が変わらない読み込み直し（1000 件）
  const unchanged: number[] = [];
  for (let i = 0; i < 20; i++) {
    unchanged.push(await clickReload());
    await settle();
  }
  // 1件だけ変わった読み込み直し
  const agent = $<HTMLInputElement>('[data-testid=reload-agent]');
  agent.click();
  await settle();
  const oneChanged: number[] = [];
  for (let i = 0; i < 20; i++) {
    oneChanged.push(await clickReload());
    await settle();
  }
  agent.click();
  await settle();
  // 1000 件ある一覧の中ほどで分割（Enter）
  const t0 = performance.now();
  for (let i = 0; i < 50; i++) {
    const el = spans()[500];
    focusAt(el, 2);
    const n = spans().length;
    key(el, 'Enter');
    while (spans().length === n) await Promise.resolve();
    forceLayout();
  }
  const splitMs = (performance.now() - t0) / 50;
  // 1文字入力（1000 件あるとき）
  const el = spans()[300];
  focusAt(el, el.textContent!.length);
  const t1 = performance.now();
  for (let i = 0; i < 200; i++) {
    document.execCommand('insertText', false, 'あ');
    forceLayout();
  }
  const typeMs = (performance.now() - t1) / 200;
  return {
    fill1000Ms: fill,
    reloadUnchanged1000Ms: median(unchanged),
    reloadOneChanged1000Ms: median(oneChanged),
    split1000MsPerOp: splitMs,
    type1000MsPerOp: typeMs,
    rowsAfter: rows().length,
  };
}

export async function runListCheck() {
  (window as unknown as { __yun2NoLog: boolean }).__yun2NoLog = true;
  const { checks: editing, counts: domWrites } = await editingChecks();
  const reload = await reloadChecks();
  const timing = await perf();
  return { editing, domWrites, reload, timing, visibility: document.visibilityState };
}
