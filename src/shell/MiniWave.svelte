<script lang="ts">
  // サイドバーの Recent の、その日の海の様子（小さな波）。残りの割合が大きいほど高い。
  // すべて終えた日（凪）は、平らな夕陽色の線にする（画面設計書 3章「サイドバー」）。
  import { wavePath } from "../lib/meter";

  let { ratio, calm }: { ratio: number; calm: boolean } = $props();
  const d = $derived(wavePath(40, 6, calm ? 0 : 0.3 + 4 * ratio, 2, 0.4));
</script>

<svg class="wave" class:calm viewBox="0 0 40 12" preserveAspectRatio="none" aria-hidden="true">
  <path {d} />
</svg>

<style>
  .wave {
    flex: none;
    width: calc(26 * var(--u));
    height: calc(10 * var(--u));
    fill: none;
    stroke: var(--wave);
    stroke-width: 1.3;
    stroke-linecap: round;
  }

  .wave path {
    vector-effect: non-scaling-stroke;
  }

  .calm {
    stroke: var(--accent);
  }
</style>
