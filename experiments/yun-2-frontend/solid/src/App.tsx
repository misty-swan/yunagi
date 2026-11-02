import { For, createSignal, onCleanup, onMount } from 'solid-js';
import { createStore } from 'solid-js/store';
import { createImeGuard, describeEvent, loggingEnabled } from './shared/ime-guard';
import { createEditor } from './shared/editor';

type Todo = { id: number; text: string; done: boolean };

export default function App() {
  const [todos, setTodos] = createStore<Todo[]>([]);
  const [draft, setDraft] = createSignal('');
  const [log, setLog] = createSignal<string[]>([]);
  let nextId = 1;
  let editorEl!: HTMLDivElement;
  const guard = createImeGuard();
  const left = () => todos.filter((t) => !t.done).length;

  function record(tag: string, e: Event, verdict?: string) {
    if (!loggingEnabled()) return;
    setLog((l) => [`[${tag}] ${describeEvent(e, verdict)}`, ...l.slice(0, 49)]);
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key !== 'Enter') return record('todo', e);
    const reason = guard.ignoreReason(e);
    if (reason) return record('todo', e, `無視（${reason}）`);
    e.preventDefault();
    const text = draft().trim();
    if (!text) return record('todo', e, '空なので追加しない');
    setTodos(todos.length, { id: nextId++, text, done: false });
    setDraft('');
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
    onCleanup(() => {
      types.forEach((t) => editorEl.removeEventListener(t, handler, true));
      view.destroy();
    });
  });

  return (
    <div class="layout">
      <section class="col">
        <h2 class="label">TODO</h2>
        <input
          class="todo-input"
          data-testid="todo-input"
          placeholder="今日やることを書く（Enter）"
          value={draft()}
          onInput={(e) => setDraft(e.currentTarget.value)}
          onKeyDown={onKeyDown}
          onCompositionStart={(e) => record('todo', e)}
          onCompositionEnd={onCompositionEnd}
        />
        <ul class="todo-list">
          <For each={todos}>
            {(todo, i) => (
              <li class="todo-item" data-testid="todo-item" data-done={String(todo.done)}>
                <input
                  type="checkbox"
                  data-testid="todo-toggle"
                  checked={todo.done}
                  onChange={(e) => setTodos(i(), 'done', e.currentTarget.checked)}
                />
                <span class="text">{todo.text}</span>
              </li>
            )}
          </For>
        </ul>
        <div class="meta">
          <span>
            {left()} left / {todos.length}
          </span>
          <button data-testid="clear-all" onClick={() => setTodos([])}>
            すべて消す
          </button>
        </div>
        <h2 class="label" style={{ 'margin-top': '16px' }}>
          Event log
        </h2>
        <pre class="log">{log().join('\n')}</pre>
      </section>
      <section class="col">
        <h2 class="label">Note</h2>
        <div class="editor" ref={editorEl} />
      </section>
    </div>
  );
}
