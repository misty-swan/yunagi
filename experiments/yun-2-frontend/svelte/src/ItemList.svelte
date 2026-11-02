<script lang="ts">
  // その場で編集する項目の一覧（prototype/app.js の itemEl・splitAt・deleteItem・moveFocus と同じ考え方）
  import { tick } from 'svelte';
  import { core, newItemId, type Item } from './shared/core';
  import { caretOffset, focusItem, focusedItemId, onFirstLine, onLastLine } from './shared/caret';
  import { createImeGuard } from './shared/ime-guard';

  let { onlog }: { onlog: (tag: string, e: Event, verdict?: string) => void } = $props();

  let items = $state<Item[]>(core.load());
  let reloads = $state(0);
  let auto = $state(false);
  let withAgent = $state(false);
  const guard = createImeGuard();

  const save = () => core.save(items);

  // コアから読み込み直す。ID は同じで中身は新しいオブジェクトの配列に置き換える
  function reload() {
    if (withAgent) core.agentEdit(focusedItemId());
    items = core.load();
    reloads++;
  }

  $effect(() => {
    if (!auto) return;
    const timer = setInterval(reload, 3000);
    return () => clearInterval(timer);
  });

  // Svelte は DOM の更新を次のマイクロタスクにまとめるので、更新を待ってからフォーカスする
  async function focusAfterUpdate(id: string, offset: number | 'end') {
    await tick();
    focusItem(id, offset);
  }

  function add() {
    const n = { id: newItemId(), text: '' };
    items.push(n);
    save();
    focusAfterUpdate(n.id, 0);
  }

  function onKeydown(e: KeyboardEvent, i: number) {
    const el = e.currentTarget as HTMLElement;
    const reason = guard.ignoreReason(e);
    if (reason) {
      if (reason === 'afterCompositionEnd') e.preventDefault(); // 確定の Enter で改行させない
      return onlog('list', e, `無視（${reason}）`);
    }
    if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
    const it = items[i];
    if (e.key === 'Enter') {
      e.preventDefault();
      const off = caretOffset(el);
      const n = { id: newItemId(), text: '' };
      if (off === 0 && it.text.length > 0) {
        items.splice(i, 0, n); // 行頭：直上に追加
        onlog('list', e, '直上に追加');
      } else {
        n.text = it.text.slice(off); // 行の途中：2件に分ける（行末なら直下に空の項目）
        it.text = it.text.slice(0, off);
        items.splice(i + 1, 0, n);
        onlog('list', e, '分割');
      }
      save();
      focusAfterUpdate(n.id, 0);
    } else if (e.key === 'Backspace' && it.text === '' && getSelection()?.isCollapsed) {
      e.preventDefault();
      items.splice(i, 1); // 空の項目：削除して直前へ
      save();
      const prev = items[Math.max(0, i - 1)];
      if (prev) focusAfterUpdate(prev.id, 'end');
      onlog('list', e, '削除');
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
</script>

<div class="meta">
  <button data-testid="reload" onclick={reload}>読み込み直す</button>
  <label><input type="checkbox" data-testid="reload-auto" bind:checked={auto} /> 3秒ごと</label>
  <label><input type="checkbox" data-testid="reload-agent" bind:checked={withAgent} /> エージェントの変更を混ぜる</label>
  <span data-testid="reload-count">reloads {reloads}</span>
</div>
<ul class="item-list">
  {#each items as item, i (item.id)}
    <li class="item-row" data-item-id={item.id}>
      <span
        class="txt"
        data-testid="item-text"
        contenteditable="plaintext-only"
        spellcheck="false"
        bind:textContent={item.text}
        oninput={save}
        onkeydown={(e) => onKeydown(e, i)}
        onbeforeinput={onBeforeInput}
        oncompositionstart={(e) => onlog('list', e)}
        oncompositionend={(e) => {
          guard.onCompositionEnd();
          onlog('list', e);
        }}
      ></span>
    </li>
  {/each}
</ul>
<div class="meta"><button data-testid="item-add" onclick={add}>項目を追加</button></div>
