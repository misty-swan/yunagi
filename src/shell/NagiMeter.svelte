<script lang="ts">
  // 凪メーター（画面設計書 3章「凪メーター」）：上部のバーの下の、空と海の景色。
  // 整理モードは高さ 112px、ノートモードは 58px の小さい版（時刻と残り件数だけ）。動きは付けない。
  // 景色は飾りで、読み上げにはまとめた説明（meterSummary）を渡す。
  import type { Counts } from "../lib/bindings";
  import { t } from "../lib/i18n";
  import { hhmm, wavePath, type MeterScene } from "../lib/meter";
  import CarryIcon from "./CarryIcon.svelte";

  let { scene, counts, compact }: { scene: MeterScene; counts: Counts; compact: boolean } = $props();

  const m = $derived(t());
  // 大きさ（本文 15px のときの px。CSS で --u を掛ける）
  const g = $derived(
    compact
      ? { height: 58, horizon: 34, sun: 14, rise: 12, waveScale: 0.55, pathW: 26, pathStep: 6, pathGap: 4 }
      : { height: 112, horizon: 72, sun: 22, rise: 30, waveScale: 1, pathW: 44, pathStep: 8, pathGap: 6 },
  );
  /** 太陽の中心の横の位置（%）。端に寄りすぎないよう 4%〜96% にする。 */
  const x = $derived(4 + scene.progress * 92);
  /** 太陽の上端（水平線より上が正の高さを引く）。沈んだら半分だけ水平線の下に出る。 */
  const sunTop = $derived(
    g.horizon - g.sun - (scene.sunk ? -g.sun / 2 : Math.sin(Math.PI * Math.max(0.06, scene.progress)) * g.rise),
  );
  const waveH = $derived(g.height - g.horizon - 2);
  const amp = $derived(scene.total === 0 || scene.calm ? 0 : (0.6 + 5 * scene.leftRatio) * g.waveScale);
  /** 3本の波：高さの位置（割合）、振幅の割合、波の数、位相、濃さ。 */
  const WAVES = [
    [0.22, 1, 9, 0, 0.85],
    [0.52, 0.7, 7, 1.3, 0.5],
    [0.8, 0.45, 11, 2.1, 0.28],
  ] as const;
  const timeLabel = $derived(
    scene.time.kind === "now"
      ? hhmm(scene.time.minutes)
      : scene.time.kind === "dayEnd"
        ? m.labels.meter.dayEnd
        : m.labels.meter.notYet,
  );
  const summary = $derived(
    m.meterSummary(scene.left, counts.leftTodos, counts.leftBlockers, counts.done, counts.todos, counts.carried),
  );
  const u = (px: number) => `calc(${px} * var(--u))`;
</script>

<div class="meter" class:compact role="img" aria-label={summary} style:height={u(g.height)}>
  <div class="sea" style:top={u(g.horizon)}></div>
  <div class="sun-clip" style:height={u(g.horizon)}>
    <div
      class="sun"
      style:left="{x}%"
      style:top={u(sunTop)}
      style:width={u(g.sun)}
      style:height={u(g.sun)}
      style:margin-left={u(-g.sun / 2)}
    ></div>
  </div>
  <div class="horizon" style:top={u(g.horizon)}></div>
  <svg
    class="waves"
    viewBox="0 0 1000 {waveH}"
    preserveAspectRatio="none"
    aria-hidden="true"
    style:top={u(g.horizon + 2)}
    style:height={u(waveH)}
  >
    {#each WAVES as [y, a, k, phase, opacity], i (i)}
      <path d={wavePath(1000, waveH * y, amp * a, k, phase)} stroke-width="1.2" {opacity} />
    {/each}
  </svg>
  {#each { length: scene.paths }, i (i)}
    {@const w = Math.max(4, g.pathW - i * g.pathStep)}
    <div
      class="path"
      style:left="{x}%"
      style:top={u(g.horizon + 5 + i * g.pathGap)}
      style:width={u(w)}
      style:margin-left={u(-w / 2)}
      style:opacity={Math.max(0.12, 0.75 - i * 0.14)}
    ></div>
  {/each}

  {#if compact}
    <div class="info" aria-hidden="true">
      <span class="time">{timeLabel}</span>
      {#if scene.calm}
        <b>{m.labels.meter.nagi}</b>
      {:else}
        <span><b>{scene.left}</b> {m.labels.meter.left}</span>
      {/if}
    </div>
  {:else}
    <div class="info" aria-hidden="true">
      <div class="time">{timeLabel}</div>
      <div class="left">
        {#if scene.calm}
          <span class="num calm">{m.labels.meter.nagi}</span>
        {:else}
          <span class="num">{scene.left}</span>
          <span class="unit">{m.labels.meter.left}</span>
        {/if}
        {#if scene.total > 0}
          <span class="sub">
            {#if !scene.calm}
              {m.labels.meter.todo} <b>{counts.leftTodos}</b> · {m.labels.meter.blockers}
              <b>{counts.leftBlockers}</b> ·
            {/if}
            {m.labels.meter.done} <b>{counts.done}/{counts.todos}</b>
            {#if !scene.calm && counts.carried > 0}
              · <CarryIcon /> {m.labels.meter.carried} <b>{counts.carried}</b>
            {/if}
          </span>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .meter {
    position: relative;
    flex: none;
    margin: 0 var(--pad);
  }

  .sea {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    background: linear-gradient(180deg, var(--sea-tint), transparent);
    pointer-events: none;
  }

  .sun-clip {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    overflow: hidden;
    pointer-events: none;
  }

  .sun {
    position: absolute;
    border-radius: 50%;
    background: radial-gradient(circle at 50% 42%, var(--sun-core), var(--accent) 72%);
    box-shadow:
      0 0 0 6px var(--sun-halo),
      0 0 36px 10px var(--sun-glow);
  }

  .horizon {
    position: absolute;
    left: 0;
    right: 0;
    height: 1px;
    background: linear-gradient(90deg, transparent, var(--horizon) 6%, var(--horizon) 94%, transparent);
  }

  .waves {
    position: absolute;
    left: 0;
    width: 100%;
    fill: none;
    stroke: var(--wave);
    stroke-linecap: round;
    pointer-events: none;
  }

  .waves path {
    vector-effect: non-scaling-stroke;
  }

  .path {
    position: absolute;
    height: 2px;
    border-radius: 2px;
    background: var(--accent);
    pointer-events: none;
  }

  .info {
    position: absolute;
    left: 0;
    top: calc(4 * var(--u));
  }

  .time {
    font-size: var(--fs-label);
    font-weight: 500;
    letter-spacing: 0.24em;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    text-transform: uppercase;
  }

  .left {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin-top: 2px;
    white-space: nowrap;
  }

  .num {
    font-size: calc(32 * var(--u));
    font-weight: 200;
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }

  .num.calm {
    font-size: calc(22 * var(--u));
    font-weight: 300;
    letter-spacing: 0.3em;
    color: var(--accent-strong);
  }

  .unit {
    font-size: var(--fs-label);
    font-weight: 600;
    letter-spacing: 0.24em;
    color: var(--text-muted);
    text-transform: uppercase;
  }

  .sub {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-left: 6px;
    font-size: var(--fs-meta);
    color: var(--text-muted);
    white-space: nowrap;
  }

  .sub b,
  .compact b {
    font-weight: 500;
    color: var(--text);
  }

  .compact .info {
    top: calc(6 * var(--u));
    display: flex;
    align-items: baseline;
    gap: 12px;
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }
</style>
