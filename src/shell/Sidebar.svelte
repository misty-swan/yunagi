<script lang="ts">
  // サイドバー（画面設計書 3章「サイドバー」）：ロゴ、ナビ（Today・Archive・Agents）、Recent、
  // 下端（Settings と状態）。幅 240px。
  import type { LogicalDate, Overview } from "../lib/bindings";
  import { shortDate } from "../lib/dates";
  import { t } from "../lib/i18n";
  import { keyLabel } from "../lib/keys";
  import { progressOf } from "../lib/meter";
  import Logo from "./Logo.svelte";
  import MiniWave from "./MiniWave.svelte";
  import type { ShellCommand } from "./commands";

  let {
    overview,
    date,
    saveFailed,
    mac,
    onShow,
    onToday,
    onCommand,
  }: {
    overview: Overview | null;
    /** 表示している日。 */
    date: LogicalDate | null;
    saveFailed: boolean;
    mac: boolean;
    onShow: (date: LogicalDate) => void;
    onToday: () => void;
    onCommand: (c: ShellCommand) => void;
  } = $props();

  const m = $derived(t());
  const today = $derived(overview?.recent[0]);
  const todayLeft = $derived(today ? progressOf(today.counts).left : 0);
  const unseen = $derived(overview?.unseenChanges ?? 0);
</script>

<aside class="side" aria-label={m.sidebar}>
  <div class="logo"><Logo /></div>

  <nav class="nav">
    <button
      type="button"
      class="nav-btn"
      aria-current={date !== null && date === overview?.today ? "page" : undefined}
      onclick={onToday}
    >
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <path d="M2 11h12M5.2 11a2.8 2.8 0 0 1 5.6 0M8 4.2v1.4M3.9 6.1l1 1M12.1 6.1l-1 1" />
      </svg>
      <span class="nl">{m.labels.nav.today}</span>
      {#if todayLeft > 0}
        <span class="nm" aria-label={m.leftToday(todayLeft)}>{todayLeft}</span>
      {/if}
    </button>
    <button type="button" class="nav-btn" onclick={() => onCommand("archive")}>
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <path d="M3 4.5h10v9H3zM3 7.5h10M5.8 3v3M10.2 3v3" />
      </svg>
      <span class="nl">{m.labels.nav.archive}</span>
    </button>
    <button type="button" class="nav-btn" onclick={() => onCommand("agents")}>
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 2.5 13.5 8 8 13.5 2.5 8z" /></svg>
      <span class="nl">{m.labels.nav.agents}</span>
      {#if unseen > 0}
        <span class="nm agent" aria-label={m.unseenChanges(unseen)}>{unseen}</span>
      {/if}
    </button>
  </nav>

  <h2 class="side-h label">{m.labels.recent}</h2>
  <ul class="recent">
    {#each overview?.recent ?? [] as day (day.date)}
      {@const c = day.counts}
      {@const p = progressOf(c)}
      <li>
        <button
          type="button"
          class="recent-btn"
          aria-current={day.date === date ? "date" : undefined}
          onclick={() => onShow(day.date)}
        >
          <MiniWave ratio={p.leftRatio} calm={p.calm} />
          <span class="rl">
            {shortDate(day.date)}
            {#if day.date === overview?.today}<span class="rt">{m.labels.today}</span>{/if}
          </span>
          {#if c.todos > 0}<span class="rc">{c.done}/{c.todos}</span>{/if}
        </button>
      </li>
    {/each}
  </ul>

  <div class="fill"></div>

  <button type="button" class="nav-btn" onclick={() => onCommand("settings")}>
    <svg viewBox="0 0 16 16" aria-hidden="true">
      <path
        d="M8 5.5a2.5 2.5 0 1 1 0 5 2.5 2.5 0 0 1 0-5zM8 1.8v2M8 12.2v2M1.8 8h2M12.2 8h2M3.6 3.6 5 5M11 11l1.4 1.4M3.6 12.4 5 11M11 5l1.4-1.4"
      />
    </svg>
    <span class="nl">{m.labels.nav.settings}</span>
    <kbd>{keyLabel("settings", mac)}</kbd>
  </button>

  <div class="status" role="status">
    {#if saveFailed}
      <div class="warn"><span class="dot warn"></span>{m.saveFailed}</div>
    {:else}
      <div><span class="dot hollow"></span>{m.labels.savedLocally}</div>
    {/if}
  </div>
</aside>

<style>
  .side {
    flex: none;
    width: var(--side-w);
    display: flex;
    flex-direction: column;
    padding: calc(18 * var(--u)) calc(14 * var(--u)) calc(14 * var(--u));
    background: var(--side);
    border-right: 1px solid var(--rule);
    overflow: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--rule) transparent;
  }

  .logo {
    padding: 2px 8px calc(20 * var(--u));
  }

  .nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .nav-btn {
    display: flex;
    align-items: center;
    gap: calc(10 * var(--u));
    width: 100%;
    padding: calc(7 * var(--u)) calc(10 * var(--u));
    border: 0;
    border-radius: calc(8 * var(--u));
    background: transparent;
    color: var(--text-muted);
    font-size: calc(14 * var(--u));
    text-align: left;
    cursor: pointer;
  }

  .nav-btn:hover {
    background: var(--hover);
  }

  .nav-btn[aria-current="page"] {
    background: var(--hover);
    color: var(--text);
    font-weight: 600;
  }

  .nav-btn svg {
    flex: none;
    width: calc(16 * var(--u));
    height: calc(16 * var(--u));
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .nl {
    flex: 1;
  }

  .nm {
    font-size: calc(11.5 * var(--u));
    font-weight: 500;
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .nm.agent {
    color: var(--agent);
  }

  .side-h {
    margin: calc(24 * var(--u)) 0 0;
    padding: 0 10px 6px;
  }

  .recent {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .recent-btn {
    display: flex;
    align-items: center;
    gap: calc(10 * var(--u));
    width: 100%;
    padding: calc(5 * var(--u)) calc(10 * var(--u));
    border: 0;
    border-radius: calc(8 * var(--u));
    background: transparent;
    color: var(--text-muted);
    font-size: var(--fs-small);
    text-align: left;
    cursor: pointer;
    font-variant-numeric: tabular-nums;
  }

  .recent-btn:hover {
    background: var(--hover);
  }

  .recent-btn[aria-current="date"] {
    background: var(--hover);
    color: var(--text);
  }

  .rl {
    flex: 1;
    white-space: nowrap;
  }

  .rt {
    margin-left: 8px;
    font-size: calc(11 * var(--u));
    color: var(--accent-strong);
  }

  .rc {
    font-size: calc(11.5 * var(--u));
    color: var(--text-muted);
  }

  .fill {
    flex: 1;
    min-height: calc(16 * var(--u));
  }

  .status {
    margin-top: 10px;
    padding: 10px 10px 2px;
    border-top: 1px solid var(--rule);
    font-size: calc(11.5 * var(--u));
    color: var(--text-muted);
    line-height: 1.9;
  }

  .status div {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .dot.hollow {
    border: 1px solid var(--text-muted);
  }

  .warn {
    color: var(--warn);
  }

  .dot.warn {
    background: var(--warn);
  }
</style>
