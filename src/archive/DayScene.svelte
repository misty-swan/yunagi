<script lang="ts">
  // 週のカードの、その日の終わりの海（画面設計書 9章）：太陽・波・光の道を小さく描く。凪メーター
  // （src/shell/NagiMeter.svelte）と同じ景色を、同じ決まり（src/lib/meter.ts）で縮めたもの。
  // 今日のカードは現在の様子、過去の日は終業の位置、未来の日は何も描かない（薄く出す）。
  import { wavePath, type MeterScene } from "../lib/meter";

  let { scene, future }: { scene: MeterScene; future: boolean } = $props();

  // 大きさ（本文 15px のときの px。CSS で --u を掛ける）
  const HEIGHT = 70;
  const HORIZON = 42;
  const SUN = 12;
  const RISE = 18;
  const x = $derived(4 + scene.progress * 92);
  const sunTop = $derived(
    HORIZON - SUN - (scene.sunk ? -SUN / 2 : Math.sin(Math.PI * Math.max(0.06, scene.progress)) * RISE),
  );
  const waveH = HEIGHT - HORIZON - 4;
  const amp = $derived(scene.total === 0 || scene.calm ? 0 : 0.4 + 5 * scene.leftRatio);
  const u = (px: number) => `calc(${px} * var(--u))`;
</script>

<div class="scene" aria-hidden="true" style:height={u(HEIGHT)}>
  {#if !future && scene.total > 0}
    <div class="sun-clip" style:height={u(HORIZON)}>
      <div
        class="sun"
        style:left="{x}%"
        style:top={u(sunTop)}
        style:width={u(SUN)}
        style:height={u(SUN)}
        style:margin-left={u(-SUN / 2)}
      ></div>
    </div>
  {/if}
  <div class="horizon" style:top={u(HORIZON)}></div>
  <svg
    viewBox="0 0 200 {waveH}"
    preserveAspectRatio="none"
    style:top={u(HORIZON + 2)}
    style:height={u(waveH)}
  >
    <path d={wavePath(200, waveH * 0.3, amp, 4, 0.3)} stroke-width="1.1" opacity="0.8" />
    <path d={wavePath(200, waveH * 0.7, amp * 0.6, 5, 1.6)} stroke-width="1" opacity="0.4" />
  </svg>
  {#if !future}
    {#each { length: scene.paths }, i (i)}
      {@const w = Math.max(4, 26 - i * 5)}
      <div
        class="path"
        style:left="{x}%"
        style:top={u(HORIZON + 4 + i * 4)}
        style:width={u(w)}
        style:margin-left={u(-w / 2)}
        style:opacity={Math.max(0.15, 0.7 - i * 0.14)}
      ></div>
    {/each}
  {/if}
</div>

<style>
  .scene {
    position: relative;
    border-radius: 8px;
    overflow: hidden;
    background: linear-gradient(180deg, var(--sky-1) 0%, var(--sky-2) 60%, var(--bg) 60%, var(--bg) 100%);
  }

  .sun-clip {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    overflow: hidden;
  }

  .sun {
    position: absolute;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 14px 4px var(--sun-glow);
  }

  .horizon {
    position: absolute;
    left: 6%;
    right: 6%;
    height: 1px;
    background: var(--horizon);
  }

  svg {
    position: absolute;
    left: 0;
    width: 100%;
    fill: none;
    stroke: var(--wave);
    stroke-linecap: round;
  }

  svg path {
    vector-effect: non-scaling-stroke;
  }

  .path {
    position: absolute;
    height: 2px;
    border-radius: 2px;
    background: var(--accent);
  }
</style>
