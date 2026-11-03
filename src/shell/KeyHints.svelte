<script lang="ts">
  // キーの案内（画面設計書 3章「キーの案内」）：画面下端に1行で、その画面の主なキーを `⌘K Search` のように。
  // 整理・ノートで内容を変える（ふりかえりの分は、その画面の課題で足す）。
  import { t } from "../lib/i18n";
  import { keyLabel, modEnterLabel, modShiftLabel } from "../lib/keys";
  import type { Mode } from "../lib/shell.svelte";

  let { mode, mac }: { mode: Mode; mac: boolean } = $props();

  const m = $derived(t());
  const hints = $derived.by((): [string, string][] => {
    const h = m.labels.hint;
    const day = `${keyLabel("prevDay", mac)} ${keyLabel("nextDay", mac)}`;
    return mode === "note"
      ? [
          [modShiftLabel("T", mac), h.toTodo],
          [modShiftLabel("B", mac), h.toBlockers],
          [modShiftLabel("H", mac), h.heading],
          [keyLabel("toggleMode", mac), h.board],
          [keyLabel("sidebar", mac), h.sidebar],
        ]
      : [
          [keyLabel("palette", mac), h.search],
          [keyLabel("toggleMode", mac), h.note],
          [modEnterLabel(mac), h.done],
          [day, h.day],
          [keyLabel("archive", mac), h.archive],
          [keyLabel("sidebar", mac), h.sidebar],
        ];
  });
</script>

<footer class="hints">
  <ul aria-label={m.keys}>
    {#each hints as [key, label] (label)}
      <li><kbd>{key}</kbd> {label}</li>
    {/each}
  </ul>
</footer>

<style>
  .hints {
    position: relative;
    flex: none;
    padding: 10px var(--pad) 16px;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 24px;
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  li {
    display: inline-flex;
    align-items: baseline;
    gap: 8px;
    white-space: nowrap;
  }

  kbd {
    color: var(--text);
    font-weight: 500;
  }
</style>
