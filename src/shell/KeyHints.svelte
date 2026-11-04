<script lang="ts">
  // キーの案内（画面設計書 3章「キーの案内」）：画面下端に1行で、その画面の主なキーを `⌘K Search` のように。
  // 整理・ノート・ふりかえりで内容を変える。
  import { t } from "../lib/i18n";
  import { keyLabel } from "../lib/keys";
  import type { Mode } from "../lib/shell.svelte";

  let { mode, archive = false, mac }: { mode: Mode; archive?: boolean; mac: boolean } = $props();

  const m = $derived(t());
  const hints = $derived.by((): [string, string][] => {
    const h = m.labels.hint;
    // 割当が使えない操作（同じキーをほかの操作が使っている）は、案内に出さない
    const day = [keyLabel("prevDay", mac), keyLabel("nextDay", mac)].filter((k) => k !== "").join(" ");
    const list: [string, string][] = archive
      ? [
          ["← →", h.day],
          [day, h.week],
          ["↵", h.open],
          ["Esc", h.back],
        ]
      : mode === "note"
        ? [
            [keyLabel("toTodo", mac), h.toTodo],
            [keyLabel("toBlocker", mac), h.toBlockers],
            [keyLabel("timeHeading", mac), h.heading],
            [keyLabel("toggleMode", mac), h.board],
            [keyLabel("sidebar", mac), h.sidebar],
          ]
        : [
            [keyLabel("palette", mac), h.search],
            [keyLabel("toggleMode", mac), h.note],
            [keyLabel("toggle", mac), h.done],
            [day, h.day],
            [keyLabel("archive", mac), h.archive],
            [keyLabel("sidebar", mac), h.sidebar],
          ];
    return list.filter(([key]) => key !== "");
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
