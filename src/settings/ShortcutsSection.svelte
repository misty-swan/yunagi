<script lang="ts">
  // 設定の「操作」区分の、ショートカット割当（操作仕様書 1章・2章、画面設計書 7章）。
  // 操作を効く範囲ごとに並べ、割当のボタンを押してから、割り当てるキーを押す。割当を持たない操作（初期割当の
  // まま）も同じ並びに出す。変えたものには「初期値に戻す」を出す。
  import { tick } from "svelte";
  import { globalShortcut } from "../lib/global_shortcut.svelte";
  import { t } from "../lib/i18n";
  import { currentKeymap, keyLabel } from "../lib/keys";
  import { ACTIONS, type ActionId, type Scope } from "../lib/keymap";
  import { prefs } from "../lib/prefs.svelte";
  import type { ShortcutEditor } from "../lib/shortcuts_edit.svelte";

  let { editor, mac }: { editor: ShortcutEditor; mac: boolean } = $props();

  const o = $derived(t().settings.operation);
  const scopes: Scope[] = ["global", "common", "board", "note"];
  const keymap = $derived(currentKeymap(mac));
  const changedAny = $derived(Object.keys(prefs.shortcuts).length > 0);

  async function start(id: ActionId) {
    editor.start(id);
    await tick();
  }

  function onKey(e: KeyboardEvent) {
    if (!editor.onKey(e)) return;
    // 取り込んだキーは、ブラウザの動き（タブの移動など）にも、ほかの画面の操作にもしない
    e.preventDefault();
    e.stopPropagation();
  }

  function reason(id: ActionId): string | null {
    const msg = editor.message;
    if (msg === null || msg.id !== id) return null;
    return "conflict" in msg ? o.conflict(o.actions[msg.conflict]) : o.problems[msg.problem];
  }

  /** 保存してある割当が使えないとき・初期割当が使えないときの注意。OS 全体の操作は、OS への登録の状態も。 */
  function warning(id: ActionId): string | null {
    const r = keymap.get(id);
    if (r === undefined) return null;
    if (r.status === "lost" && r.takenBy !== undefined) return o.lost(o.actions[r.takenBy]);
    const own = r.status === "ignored" ? o.ignored : null;
    return id === "bringToFront" ? [own, registration()].filter(Boolean).join(" ") || null : own;
  }

  /** OS 全体の操作を OS に登録できなかった理由と、Wayland の案内（登録できても、Wayland では出す）。 */
  function registration(): string | null {
    const s = globalShortcut.status;
    // 登録していない（割当が使えない）ときは、案内も出さない
    if (!s || s.state === "off") return null;
    const reason =
      s.state === "taken" ? o.global.taken : s.state === "failed" ? o.global.failed : s.state === "unavailable" ? o.global.unavailable : null;
    return [reason, s.wayland ? o.global.wayland : null].filter(Boolean).join(" ") || null;
  }
</script>

<div class="shortcuts" role="group" aria-label={o.shortcuts}>
  <p class="note head">{o.shortcutsNote}</p>
  {#each scopes as scope (scope)}
    <h4 class="scope">{o.scopes[scope]}</h4>
    {#each ACTIONS.filter((a) => a.scope === scope) as a (a.id)}
      {@const key = keyLabel(a.id, mac)}
      {@const bad = reason(a.id) ?? warning(a.id)}
      <div class="srow" data-action={a.id}>
        <span class="sname">{o.actions[a.id]}</span>
        <div class="scontrol">
          <button
            type="button"
            class="key-btn"
            class:capturing={editor.capturing === a.id}
            aria-label={o.changeKey(o.actions[a.id], key)}
            onclick={() => void start(a.id)}
            onkeydown={onKey}
            onblur={() => editor.capturing === a.id && editor.cancel()}
          >
            {#if editor.capturing === a.id}
              {o.pressKey}
            {:else if key !== ""}
              <kbd>{key}</kbd>
            {:else}
              —
            {/if}
          </button>
          {#if prefs.shortcuts[a.id] !== undefined}
            <button type="button" class="btn small" onclick={() => editor.reset(a.id)}>{o.reset}</button>
          {/if}
        </div>
        {#if bad}<p class="bad" role="status">{bad}</p>{/if}
      </div>
    {/each}
  {/each}
  {#if changedAny}
    <div class="all">
      <button type="button" class="btn" onclick={() => editor.resetAll()}>{o.resetAll}</button>
    </div>
  {/if}
</div>

<style>
  .shortcuts {
    padding: calc(12 * var(--u)) 0;
    border-bottom: 1px solid var(--rule);
  }

  .head {
    margin: 0 0 calc(8 * var(--u));
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .scope {
    margin: calc(14 * var(--u)) 0 calc(4 * var(--u));
    font-size: var(--fs-label);
    font-weight: 600;
    letter-spacing: 0.14em;
    color: var(--text-muted);
  }

  .srow {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: calc(2 * var(--u)) calc(12 * var(--u));
    align-items: center;
    padding: calc(5 * var(--u)) 0;
  }

  .sname {
    font-size: var(--fs-small);
  }

  .scontrol {
    display: flex;
    align-items: center;
    gap: calc(8 * var(--u));
  }

  .key-btn {
    min-width: calc(120 * var(--u));
    font: inherit;
    font-size: var(--fs-small);
    color: var(--text);
    text-align: center;
    padding: calc(4 * var(--u)) calc(10 * var(--u));
    background: var(--field);
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
    cursor: pointer;
  }

  .key-btn:hover,
  .key-btn.capturing {
    border-color: var(--accent-strong);
  }

  .key-btn.capturing {
    color: var(--accent-strong);
  }

  kbd {
    font-family: inherit;
  }

  .small {
    padding: calc(3 * var(--u)) calc(10 * var(--u));
  }

  .bad {
    grid-column: 1 / -1;
    margin: 0;
    font-size: var(--fs-meta);
    color: var(--warn);
  }

  .all {
    margin-top: calc(12 * var(--u));
  }
</style>
