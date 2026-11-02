<script lang="ts">
  import { onMount } from 'svelte';
  import { createImeGuard, describeEvent, loggingEnabled } from './shared/ime-guard';
  import { createEditor } from './shared/editor';
  import ItemList from './ItemList.svelte';

  type Todo = { id: number; text: string; done: boolean };

  let todos = $state<Todo[]>([]);
  let draft = $state('');
  let log = $state<string[]>([]);
  let nextId = 1;
  let editorEl: HTMLDivElement;
  const guard = createImeGuard();
  const left = $derived(todos.filter((t) => !t.done).length);

  function record(tag: string, e: Event, verdict?: string) {
    if (!loggingEnabled()) return;
    log = [`[${tag}] ${describeEvent(e, verdict)}`, ...log.slice(0, 49)];
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key !== 'Enter') return record('todo', e);
    const reason = guard.ignoreReason(e);
    if (reason) return record('todo', e, `無視（${reason}）`);
    e.preventDefault();
    const text = draft.trim();
    if (!text) return record('todo', e, '空なので追加しない');
    todos.push({ id: nextId++, text, done: false });
    draft = '';
    record('todo', e, '追加');
  }

  function onCompositionEnd(e: CompositionEvent) {
    guard.onCompositionEnd();
    record('todo', e);
  }

  onMount(() => {
    const view = createEditor(editorEl);
    const types = ['keydown', 'compositionstart', 'compositionend'] as const;
    const handler = (e: Event) => record('editor', e);
    types.forEach((t) => editorEl.addEventListener(t, handler, true));
    return () => {
      types.forEach((t) => editorEl.removeEventListener(t, handler, true));
      view.destroy();
    };
  });
</script>

<div class="layout">
  <section class="col">
    <h2 class="label">TODO</h2>
    <input
      class="todo-input"
      data-testid="todo-input"
      placeholder="今日やることを書く（Enter）"
      bind:value={draft}
      onkeydown={onKeydown}
      oncompositionstart={(e) => record('todo', e)}
      oncompositionend={onCompositionEnd}
    />
    <ul class="todo-list">
      {#each todos as todo (todo.id)}
        <li class="todo-item" data-testid="todo-item" data-done={String(todo.done)}>
          <input type="checkbox" data-testid="todo-toggle" bind:checked={todo.done} />
          <span class="text">{todo.text}</span>
        </li>
      {/each}
    </ul>
    <div class="meta">
      <span>{left} left / {todos.length}</span>
      <button data-testid="clear-all" onclick={() => (todos = [])}>すべて消す</button>
    </div>
    <h2 class="label" style="margin-top:16px">Event log</h2>
    <pre class="log">{log.join('\n')}</pre>
  </section>
  <section class="col">
    <h2 class="label">Board</h2>
    <ItemList onlog={record} />
  </section>
  <section class="col">
    <h2 class="label">Note</h2>
    <div class="editor" bind:this={editorEl}></div>
  </section>
</div>
