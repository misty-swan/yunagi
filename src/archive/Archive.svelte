<script lang="ts">
  // ふりかえり（S-08 Archive。画面設計書 9章）：過去の日と週を見返す画面。外枠の本文に、整理モード・
  // ノートの代わりに出す。
  // 部分：This week（選んだ日を含む週の範囲と合計）、週のカード（月〜金の5枚）、Daily notes（記録のある
  // 直近の日）、選んだ日のプレビュー（数字・振り返りの全文・ノートの時刻付き見出し）。
  //
  // キー（操作仕様書 2章「ふりかえり」）：← → で日、Enter でその日を開く、Esc で戻る。Mod+[ / Mod+] の
  // 週の移動と Mod+Shift+L は外枠（AppShell）が受ける。キーはこの画面の枠で受ける（検索パレット・設定など、
  // 上に重ねた画面のキーを受けないため）。ダブルクリックでもその日を開く。
  import { tick } from "svelte";
  import type { ArchiveScreen } from "../lib/archive.svelte";
  import { reflectionBlocks, reflectionPicks, stripBullet, timeLabel } from "../lib/archive_text";
  import type { Counts, LogicalDate } from "../lib/bindings";
  import { monthDay, shortDate, weekday } from "../lib/dates";
  import { t } from "../lib/i18n";
  import { meterScene, minutesOfDay, progressOf } from "../lib/meter";
  import { prefs } from "../lib/prefs.svelte";
  import MiniWave from "../shell/MiniWave.svelte";
  import DayScene from "./DayScene.svelte";

  let {
    archive,
    narrow,
    onOpenDay,
    onClose,
  }: {
    archive: ArchiveScreen;
    /** ウィンドウが狭い（Daily notes とプレビューを縦に並べる）。 */
    narrow: boolean;
    /** その日のボードを開く（Enter・ダブルクリック・「Open ↵」）。 */
    onOpenDay: (date: LogicalDate) => void;
    /** 元の画面へ戻る（Esc）。 */
    onClose: () => void;
  } = $props();

  const m = $derived(t());
  const a = $derived(m.archive);
  const l = $derived(m.labels.archive);
  const data = $derived(archive.data);

  /** 今日のカードの太陽の時刻のため、いまの時刻を 30 秒ごとに読み直す。 */
  const CLOCK_MS = 30_000;
  let now = $state(new Date());
  $effect(() => {
    const timer = setInterval(() => (now = new Date()), CLOCK_MS);
    return () => clearInterval(timer);
  });

  let root = $state<HTMLElement | null>(null);
  // 開いたとき、日を選び直したとき（サイドバーなど）に、枠へフォーカスを置く
  $effect(() => {
    archive.focusSeq;
    void tick().then(() => root?.focus({ preventScroll: true }));
  });

  const week = $derived(data?.week ?? []);
  const today = $derived(data?.today ?? null);
  const selected = $derived(data?.date ?? null);

  const dailyRows = $derived(
    (data?.daily ?? []).map((d) => ({ ...d, picks: reflectionPicks(d.reflection), progress: progressOf(d.counts) })),
  );

  const preview = $derived(data?.preview ?? null);
  const blocks = $derived(preview ? reflectionBlocks(preview.reflection) : []);
  const empty = $derived(
    preview !== null &&
      blocks.length === 0 &&
      preview.note.length === 0 &&
      preview.counts.todos + preview.counts.carried + preview.counts.resolved + preview.counts.leftBlockers === 0,
  );

  function ratio(c: Counts): string {
    return c.todos > 0 ? `${c.done}/${c.todos}` : "";
  }

  function open(date: LogicalDate | null) {
    if (date !== null) onOpenDay(date);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.defaultPrevented || e.isComposing || e.keyCode === 229) return;
    if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      e.preventDefault();
      archive.moveDay(e.key === "ArrowLeft" ? -1 : 1);
      root?.focus({ preventScroll: true });
    } else if (e.key === "Enter") {
      // ボタンの上でも、選んだ日を開く（カードの Enter は日を選ぶのではなく開く）
      e.preventDefault();
      if (!e.repeat) open(selected);
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section
  class="archive"
  class:narrow
  bind:this={root}
  tabindex="-1"
  aria-label={a.label}
  onkeydown={onKeydown}
>
  {#if data && today && selected && preview}
    <div class="head">
      <span class="label">{l.thisWeek}</span>
      <span class="range">{shortDate(week[0].date)} — {shortDate(week[week.length - 1].date)}</span>
      <span class="sum">
        <b>{data.weekTotal.done}</b> {l.done} · <b>{data.weekTotal.carried}</b> {l.carried} ·
        <b>{data.weekTotal.resolved}</b> {l.resolved}
      </span>
    </div>

    <div class="week" role="listbox" aria-label={a.weekLabel}>
      {#each week as d (d.date)}
        {@const future = d.date > today}
        {@const scene = meterScene(d.date, today, minutesOfDay(today, now), d.counts, prefs.meterHours)}
        <button
          type="button"
          class="card"
          class:future
          role="option"
          aria-selected={d.date === selected}
          aria-disabled={future}
          aria-label={a.card(shortDate(d.date), d.counts.done, d.counts.todos, d.date === today, future)}
          onclick={() => archive.select(d.date)}
          ondblclick={() => !future && open(d.date)}
        >
          <DayScene {scene} {future} />
          <span class="cap">
            <span>{monthDay(d.date)}</span>
            <span class="wd">{weekday(d.date)}</span>
            {#if d.date === today}<span class="tg">{m.labels.today}</span>{/if}
            <span class="ct">{future ? "" : d.counts.todos > 0 ? ratio(d.counts) : "—"}</span>
          </span>
        </button>
      {/each}
    </div>

    <div class="cols">
      <section class="daily" aria-label={a.dailyLabel}>
        <div class="label">{l.dailyNotes}</div>
        {#each dailyRows as d (d.date)}
          <button
            type="button"
            class="entry"
            aria-pressed={d.date === selected}
            onclick={() => archive.select(d.date)}
            ondblclick={() => open(d.date)}
          >
            <span class="eh">
              <MiniWave ratio={d.progress.leftRatio} calm={d.progress.calm} />
              <b>{shortDate(d.date)}</b>
              {#if d.date === today}<span class="tg">{m.labels.today}</span>{/if}
              <span class="ct">{ratio(d.counts)}</span>
            </span>
            <span class="el"><span class="tag">{l.good}</span>{d.picks.good ?? a.noLine}</span>
            <span class="el muted"><span class="tag st">{l.stuck}</span>{d.picks.stuck ?? a.noLine}</span>
          </button>
        {:else}
          <p class="none">{a.noDays}</p>
        {/each}
      </section>

      <section class="prev" aria-label={a.previewLabel}>
        <div class="ph">
          <span class="pd">{monthDay(selected)}<span class="wd">{weekday(selected)}</span></span>
          <button type="button" class="open" aria-label={a.openDay(shortDate(selected))} onclick={() => open(selected)}>
            {l.open}<kbd>↵</kbd>
          </button>
        </div>
        {#if empty}
          <p class="none">{a.noRecord}</p>
        {:else}
          <div class="ps">
            <b>{preview.counts.done}</b>/{preview.counts.todos} {l.done} · <b>{preview.counts.carried}</b>
            {l.carried} · <b>{preview.counts.resolved}</b> {l.resolved}
          </div>
          <div class="pcols">
            <div>
              <div class="label">{l.reflection}</div>
              {#each blocks as b, i (i)}
                <div class={b.kind === "heading" ? "ph2" : "pl"}>{b.text}</div>
              {:else}
                <div class="pl muted">{a.noReflection}</div>
              {/each}
            </div>
            <div>
              <div class="label">{l.note}</div>
              {#each preview.note as n, i (i)}
                <div class="nh">
                  <span class="t">{timeLabel(n.hour, n.minute)}</span>
                  {#if n.title !== ""}<span class="n">{n.title}</span>{/if}
                </div>
                {#each n.lines as line, j (j)}
                  <div class="pl muted">{stripBullet(line)}</div>
                {/each}
              {:else}
                <div class="pl muted">{a.noNote}</div>
              {/each}
            </div>
          </div>
        {/if}
      </section>
    </div>
  {:else if archive.error}
    <p class="none" role="status">{a.loadFailed}</p>
  {/if}
</section>

<style>
  .archive {
    position: relative;
    height: 100%;
    overflow: auto;
    padding: calc(6 * var(--u)) var(--pad) calc(16 * var(--u));
    scrollbar-width: thin;
    scrollbar-color: var(--rule) transparent;
  }

  .archive:focus-visible {
    outline: none;
  }

  .head {
    display: flex;
    align-items: baseline;
    gap: 16px;
    flex-wrap: wrap;
  }

  .range {
    font-size: calc(20 * var(--u));
    font-weight: 300;
    font-variant-numeric: tabular-nums;
  }

  .sum {
    font-size: calc(12.5 * var(--u));
    color: var(--text-muted);
  }

  .sum b,
  .ps b {
    font-weight: 500;
    color: var(--text);
  }

  .week {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 12px;
    margin-top: 14px;
  }

  .card {
    display: block;
    padding: 8px;
    border-radius: 12px;
    border: 1px solid var(--rule);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .card[aria-selected="true"] {
    border-color: var(--accent-strong);
    box-shadow: 0 0 0 1px var(--accent-strong);
  }

  .card.future {
    opacity: 0.5;
    cursor: default;
  }

  .cap {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 8px 4px 2px;
    font-size: var(--fs-small);
    font-variant-numeric: tabular-nums;
  }

  .wd {
    color: var(--text-muted);
  }

  .tg {
    font-size: calc(11 * var(--u));
    color: var(--accent-strong);
  }

  .ct {
    margin-left: auto;
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .cols {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1.25fr);
    gap: 28px;
    margin-top: 22px;
  }

  .narrow .cols {
    grid-template-columns: minmax(0, 1fr);
    gap: 20px;
  }

  .narrow .week {
    gap: 8px;
  }

  .daily {
    min-width: 0;
  }

  .entry {
    display: block;
    width: 100%;
    margin-top: 10px;
    padding: 10px 12px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .entry:hover {
    background: var(--hover);
  }

  .entry[aria-pressed="true"] {
    background: var(--sel);
    box-shadow: inset 2px 0 0 var(--accent-strong);
  }

  .eh {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: var(--fs-small);
    font-variant-numeric: tabular-nums;
  }

  .eh b {
    font-weight: 500;
  }

  .el {
    display: block;
    margin-top: 4px;
    font-size: calc(13.5 * var(--u));
    line-height: 1.7;
  }

  .el + .el {
    margin-top: 0;
  }

  .muted {
    color: var(--text-muted);
  }

  .tag {
    display: inline-block;
    min-width: 3.6em;
    margin-right: 8px;
    font-size: calc(10 * var(--u));
    font-weight: 600;
    letter-spacing: 0.16em;
    color: var(--accent-strong);
  }

  .tag.st {
    color: var(--warn);
  }

  .prev {
    min-width: 0;
    padding: 18px 22px;
    border-radius: 14px;
    border: 1px solid var(--rule);
    background: var(--surface);
  }

  .ph {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .pd {
    font-size: calc(17 * var(--u));
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }

  .pd .wd {
    margin-left: 8px;
    font-weight: 400;
  }

  .open {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: calc(30 * var(--u));
    padding: 0 12px;
    border-radius: 8px;
    border: 1px solid var(--accent-strong);
    background: transparent;
    color: var(--accent-strong);
    font: inherit;
    font-size: calc(12.5 * var(--u));
    font-weight: 600;
    cursor: pointer;
  }

  .open:hover {
    background: var(--sel);
  }

  .ps {
    margin-top: 4px;
    font-size: calc(12.5 * var(--u));
    color: var(--text-muted);
  }

  .pcols {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 24px;
    margin-top: 16px;
  }

  .narrow .pcols {
    grid-template-columns: minmax(0, 1fr);
    gap: 16px;
  }

  .ph2 {
    margin-top: 8px;
    font-size: calc(12 * var(--u));
    font-weight: 500;
    letter-spacing: 0.08em;
    color: var(--accent-strong);
  }

  .pl {
    font-size: calc(14 * var(--u));
    overflow-wrap: anywhere;
  }

  .nh {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin-top: 8px;
  }

  .nh .t {
    font-size: calc(17 * var(--u));
    font-weight: 300;
    color: var(--accent-strong);
    font-variant-numeric: tabular-nums;
  }

  .nh .n {
    font-size: calc(14 * var(--u));
    font-weight: 500;
  }

  .none {
    margin: 12px 0 0;
    color: var(--text-muted);
    font-size: var(--fs-small);
  }
</style>
