// その場で編集する項目の一覧（prototype/app.js の itemEl・splitAt・deleteItem・moveFocus と同じ考え方）
import { For, createEffect, createSignal, onCleanup } from 'solid-js';
import { createStore, produce, reconcile } from 'solid-js/store';
import { core, newItemId, type Item } from './shared/core';
import { caretOffset, focusItem, focusedItemId, onFirstLine, onLastLine } from './shared/caret';
import { createImeGuard } from './shared/ime-guard';

export default function ItemList(props: { onLog: (tag: string, e: Event, verdict?: string) => void }) {
  const [items, setItems] = createStore<Item[]>(core.load());
  const [reloads, setReloads] = createSignal(0);
  const [auto, setAuto] = createSignal(false);
  const [withAgent, setWithAgent] = createSignal(false);
  const guard = createImeGuard();

  const save = () => core.save(items);

  // コアから読み込み直す。ID は同じで中身は新しいオブジェクトの配列。
  // reconcile で ID ごとに突き合わせ、変わった値だけを更新する（行の DOM を作り直さない）
  function reload() {
    if (withAgent()) core.agentEdit(focusedItemId());
    setItems(reconcile(core.load(), { key: 'id' }));
    setReloads((n) => n + 1);
  }

  createEffect(() => {
    if (!auto()) return;
    const timer = setInterval(reload, 3000);
    onCleanup(() => clearInterval(timer));
  });

  // Solid はストアの更新で DOM が同期的に変わるので、そのままフォーカスできる
  function add() {
    const n = { id: newItemId(), text: '' };
    setItems(items.length, n);
    save();
    focusItem(n.id, 0);
  }

  function onKeyDown(e: KeyboardEvent, i: number, el: HTMLElement) {
    const reason = guard.ignoreReason(e);
    if (reason) {
      if (reason === 'afterCompositionEnd') e.preventDefault(); // 確定の Enter で改行させない
      return props.onLog('list', e, `無視（${reason}）`);
    }
    if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
    const it = items[i];
    if (e.key === 'Enter') {
      e.preventDefault();
      const off = caretOffset(el);
      const n = { id: newItemId(), text: '' };
      if (off === 0 && it.text.length > 0) {
        setItems(produce((l) => l.splice(i, 0, n))); // 行頭：直上に追加
        props.onLog('list', e, '直上に追加');
      } else {
        n.text = it.text.slice(off); // 行の途中：2件に分ける（行末なら直下に空の項目）
        const head = it.text.slice(0, off);
        setItems(
          produce((l) => {
            l[i].text = head;
            l.splice(i + 1, 0, n);
          }),
        );
        props.onLog('list', e, '分割');
      }
      save();
      focusItem(n.id, 0);
    } else if (e.key === 'Backspace' && it.text === '' && getSelection()?.isCollapsed) {
      e.preventDefault();
      setItems(produce((l) => l.splice(i, 1))); // 空の項目：削除して直前へ
      save();
      const prev = items[Math.max(0, i - 1)];
      if (prev) focusItem(prev.id, 'end');
      props.onLog('list', e, '削除');
    } else if (e.key === 'ArrowUp' && i > 0 && onFirstLine(el)) {
      e.preventDefault();
      const n = items[i - 1];
      focusItem(n.id, Math.min(caretOffset(el), n.text.length));
    } else if (e.key === 'ArrowDown' && i < items.length - 1 && onLastLine(el)) {
      e.preventDefault();
      const n = items[i + 1];
      focusItem(n.id, Math.min(caretOffset(el), n.text.length));
    }
  }

  function onBeforeInput(e: InputEvent) {
    // 改行は入れない（Enter は keydown で処理する）
    if (e.inputType === 'insertParagraph' || e.inputType === 'insertLineBreak') e.preventDefault();
  }

  return (
    <>
      <div class="meta">
        <button data-testid="reload" onClick={reload}>
          読み込み直す
        </button>
        <label>
          <input type="checkbox" data-testid="reload-auto" checked={auto()} onChange={(e) => setAuto(e.currentTarget.checked)} />{' '}
          3秒ごと
        </label>
        <label>
          <input
            type="checkbox"
            data-testid="reload-agent"
            checked={withAgent()}
            onChange={(e) => setWithAgent(e.currentTarget.checked)}
          />{' '}
          エージェントの変更を混ぜる
        </label>
        <span data-testid="reload-count">reloads {reloads()}</span>
      </div>
      <ul class="item-list">
        <For each={items}>
          {(item, i) => {
            let el!: HTMLSpanElement;
            // contenteditable の中身は、データと DOM が違うときだけ書き換える。
            // 同じなら触らない（カーソル・変換中の文字を保つため。Svelte の bind:textContent と同じ考え方）
            createEffect(() => {
              if (el.textContent !== item.text) el.textContent = item.text;
            });
            return (
              <li class="item-row" data-item-id={item.id}>
                <span
                  ref={el}
                  class="txt"
                  data-testid="item-text"
                  contentEditable="plaintext-only"
                  spellcheck={false}
                  onInput={() => {
                    setItems(i(), 'text', el.textContent ?? '');
                    save();
                  }}
                  onKeyDown={(e) => onKeyDown(e, i(), el)}
                  onBeforeInput={onBeforeInput}
                  onCompositionStart={(e) => props.onLog('list', e)}
                  onCompositionEnd={(e) => {
                    guard.onCompositionEnd();
                    props.onLog('list', e);
                  }}
                />
              </li>
            );
          }}
        </For>
      </ul>
      <div class="meta">
        <button data-testid="item-add" onClick={add}>
          項目を追加
        </button>
      </div>
    </>
  );
}
