<script lang="ts">
  // Vim の現在のモード（NORMAL / INSERT / VISUAL）を、エディタの右下に小さく出す（操作仕様書 4.1）。
  // 置く親は `position: relative` にする。Vim が無効のとき（`mode` が `null`）は何も出さない。
  import type { VimMode } from "../lib/editor_vim";

  let { mode }: { mode: VimMode | null } = $props();
</script>

{#if mode !== null}
  <div class="vim-mode" data-mode={mode}>{mode.toUpperCase()}</div>
{/if}

<style>
  .vim-mode {
    position: absolute;
    right: calc(10 * var(--u));
    bottom: calc(6 * var(--u));
    font-size: calc(10.5 * var(--u));
    font-weight: 600;
    letter-spacing: 0.14em;
    color: var(--text-faint);
    pointer-events: none;
    user-select: none;
  }

  .vim-mode[data-mode="insert"] {
    color: var(--accent-strong);
  }

  .vim-mode[data-mode="visual"] {
    color: var(--sea);
  }
</style>
