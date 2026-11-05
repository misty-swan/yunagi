<script lang="ts">
  // エージェント変更履歴（S-06。画面設計書 8章）。外枠の上に重ねるパネル（形は設定 S-05・検索パレット S-04・
  // 持ち越し S-03 と同じ）。上にエージェントごとの権限の切り替え、下に変更の一覧（新しい順に、時刻・
  // エージェント・操作・対象・変更内容の要約）。各行で「取り消す」（確かめる文をその行の下に出す）と
  // 「確認済みにする」を行える。状態と操作は AgentsScreen（src/lib/agents.svelte.ts）、権限は AgentAdmin。
  //
  // キー：Esc（確かめる文を出していれば、先にそれを閉じる）で閉じる。Tab はパネルの中だけを回る。開いている間は、
  // ほかの画面のキーを受け付けない（背後にフォーカスが落ちたときは、Esc は閉じ、ほかのキーは戻すだけ）。
  import { tick } from "svelte";
  import AgentList from "./AgentList.svelte";
  import type { AgentAdmin } from "../lib/agent_admin.svelte";
  import type { AgentsScreen } from "../lib/agents.svelte";
  import { backupTime } from "../lib/backups";
  import type { ChangeEntry } from "../lib/bindings";
  import { shortDate } from "../lib/dates";
  import { agentLabel, t } from "../lib/i18n";

  let { screen, admin }: { screen: AgentsScreen; admin: AgentAdmin } = $props();

  const m = $derived(t());
  const a = $derived(m.agents);
  const h = $derived(a.history);
  const labels = $derived(m.labels.agentsScreen);

  let root = $state<HTMLElement | null>(null);

  // 開いたら権限の一覧を読み、閉じるときは確かめている途中の操作を捨てる
  $effect(() => {
    if (!screen.open) return;
    const release = admin.retain();
    void admin.load();
    return () => {
      release();
      admin.reset();
    };
  });

  // 開いたとき・開き直したときに、パネルへフォーカスを置く。閉じたら、開く直前にフォーカスのあった所へ戻す
  let wasOpen = false;
  $effect(() => {
    screen.focusSeq;
    if (screen.open) {
      wasOpen = true;
      void tick().then(() => root?.focus({ preventScroll: true }));
      return;
    }
    if (!wasOpen) return;
    wasOpen = false;
    const back = screen.returnFocus;
    screen.returnFocus = null;
    const active = document.activeElement;
    if (back instanceof HTMLElement && back.isConnected && (active === null || active === document.body)) back.focus();
  });

  // 取り消しの確かめる文を出したら、その中のボタンへフォーカスを移す
  const flowKey = $derived(screen.flow === null ? null : `${screen.flow.entry.id}:${screen.flow.plan?.token ?? ""}:${screen.flow.problem?.kind ?? ""}`);
  $effect(() => {
    if (flowKey === null) return;
    // 初めのフォーカスは「やめる」（確定のボタンではなく）。キーボードの二度押しで、取り消しが確定しないように
    void tick().then(() => root?.querySelector<HTMLElement>(".revert [data-initial]")?.focus());
  });

  const unseenCount = $derived(screen.unseen.length);
  const agentNames = $derived(admin.agents?.map((r) => r.name) ?? []);

  function focusables(): HTMLElement[] {
    if (root === null) return [];
    return [
      ...root.querySelectorAll<HTMLElement>("button, input, select, textarea, a[href], [tabindex]:not([tabindex='-1'])"),
    ].filter((el) => !el.hasAttribute("disabled"));
  }

  function onKeydown(e: KeyboardEvent) {
    // ほかの画面のキー（window で受けるもの）へ渡さない。入力欄の編集の既定の動きは止めない
    e.stopPropagation();
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === "Tab") {
      const items = focusables();
      if (items.length === 0) return;
      const i = items.indexOf(document.activeElement as HTMLElement);
      const next = e.shiftKey ? (i <= 0 ? items.length - 1 : i - 1) : i === items.length - 1 ? 0 : i + 1;
      e.preventDefault();
      items[next].focus();
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      // 確かめる文を出していれば、先にそれを閉じる
      if (screen.flow !== null) screen.cancelRevert();
      else if (admin.ask !== null) admin.cancelAsk();
      else if (admin.removing !== null) admin.cancelRemove();
      else screen.dismiss();
    }
  }

  // 背後にフォーカスがあるときのキーも、ほかの画面の操作にしない（覆いの外へフォーカスが落ちたとき）
  function onWindowKeydown(e: KeyboardEvent) {
    if (!screen.open || root === null) return;
    if (e.target instanceof Node && root.contains(e.target)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === "Escape") screen.dismiss();
    else root.focus({ preventScroll: true });
  }

  function targetText(e: ChangeEntry): string {
    const tg = e.target;
    switch (tg.type) {
      case "item":
        return tg.item === null
          ? h.targetMissing
          : h.targetItem(tg.item.kind, shortDate(tg.item.date), tg.item.text, tg.item.deleted);
      case "note":
        return h.targetNote(shortDate(tg.date));
      case "reflection":
        return h.targetReflection(shortDate(tg.date));
    }
  }

  const noteFields = (names: string[]) => names.map((n) => h.field(n)).join("・");
</script>

<svelte:window onkeydowncapture={onWindowKeydown} />

{#snippet summary(e: ChangeEntry)}
  {#if e.summary.type === "added"}
    <p class="sum">{h.added(e.summary.text)}</p>
  {:else if e.summary.type === "fields"}
    <ul class="sum fields">
      {#each e.summary.fields as f (f.name)}
        <li><span class="fname">{h.field(f.name)}</span> {h.change(h.value(f.name, f.before), h.value(f.name, f.after))}</li>
      {/each}
    </ul>
  {:else if e.summary.type === "text"}
    <p class="sum">{h.chars(e.summary.beforeChars, e.summary.afterChars)}</p>
  {:else}
    <p class="sum muted">{h.unknownRecord}</p>
  {/if}
{/snippet}

{#snippet revertBox(e: ChangeEntry)}
  {@const f = screen.flow}
  {#if f !== null && f.entry.id === e.id}
    <div class="revert" role="group" aria-label={h.revertLabel(h.operation(e.operation))} data-revert={e.id}>
      {#if f.problem !== null}
        <p class="warn" role="alert">
          {h.problem(f.problem.kind, { date: f.problem.date === undefined ? undefined : shortDate(f.problem.date) })}
        </p>
        <div class="actions">
          <button type="button" class="btn" data-initial onclick={() => screen.cancelRevert()}>{h.revertCancel}</button>
        </div>
      {:else if f.plan === null}
        <p class="muted" role="status">{h.previewing}</p>
      {:else}
        {@const plan = f.plan}
        {#if f.changedAfterPreview}<p class="warn" role="alert">{h.changedAfterPreview}</p>{/if}
        {#if plan.state === "already_reverted"}
          <p>{h.planAlreadyReverted}</p>
        {:else if plan.state === "already_restored"}
          <p>{h.planAlreadyRestored}</p>
        {:else if plan.effect.type === "fields"}
          <p>{h.planFields}</p>
          <table class="plan">
            <thead>
              <tr><th></th><th>{h.planNow}</th><th>{h.planRestore}</th></tr>
            </thead>
            <tbody>
              {#each plan.effect.fields as fld (fld.name)}
                <tr class:since={fld.changedSince}>
                  <th scope="row">{h.field(fld.name)}</th>
                  <td>{h.value(fld.name, fld.current)}</td>
                  <td>{h.value(fld.name, fld.restore)}</td>
                  {#if fld.changedSince}<td class="flag">{h.planChangedSince}</td>{/if}
                </tr>
              {/each}
            </tbody>
          </table>
        {:else if plan.effect.type === "delete_item"}
          <p>{h.planDelete(plan.effect.text, shortDate(plan.effect.date))}</p>
          {#if plan.effect.carriedToDate !== null}<p class="muted">{h.planKeepsCarried(shortDate(plan.effect.carriedToDate))}</p>{/if}
        {:else if plan.effect.type === "text"}
          <p>{h.planText(plan.effect.doc, shortDate(plan.effect.date))}</p>
          <div class="diff">
            {#if plan.effect.removed.length > 0}
              <p class="dlabel">{h.planRemoved}</p>
              <pre class="removed">{plan.effect.removed.map((l) => `- ${l}`).join("\n")}</pre>
            {/if}
            {#if plan.effect.added.length > 0}
              <p class="dlabel">{h.planAdded}</p>
              <pre class="added">{plan.effect.added.map((l) => `+ ${l}`).join("\n")}</pre>
            {/if}
            {#if plan.effect.truncated}<p class="muted">{h.planTruncated}</p>{/if}
          </div>
        {/if}
        {#if plan.state === "apply" && plan.changedSince.length > 0}
          <p class="warn">{h.forceWarning(noteFields(plan.changedSince))}</p>
        {/if}
        <div class="actions">
          {#if plan.state === "already_reverted"}
            <button type="button" class="btn" data-initial onclick={() => screen.cancelRevert()}>{a.close}</button>
          {:else}
            <button
              type="button"
              class="btn primary"
              disabled={f.busy}
              onclick={() => void screen.confirmRevert(plan.state === "apply" && plan.changedSince.length > 0)}
            >
              {f.busy
                ? a.working
                : plan.state === "already_restored"
                  ? h.revertMark
                  : plan.changedSince.length > 0
                    ? h.revertForce
                    : h.revertOk}
            </button>
            <button type="button" class="btn" data-initial disabled={f.busy} onclick={() => screen.cancelRevert()}>
              {h.revertCancel}
            </button>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
{/snippet}

{#if screen.open}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="ov" onkeydown={onKeydown} onmousedown={(e) => e.target === e.currentTarget && screen.dismiss()}>
    <div class="panel" role="dialog" aria-modal="true" aria-label={a.label} data-agents tabindex="-1" bind:this={root}>
      <header class="head">
        <h2 class="title">{labels.title}</h2>
        <button type="button" class="btn" onclick={() => screen.dismiss()}>{a.close} <kbd>Esc</kbd></button>
      </header>
      <div class="scroll">
        <section aria-labelledby="ag-agents">
          <h3 class="label" id="ag-agents">{labels.agents}</h3>
          <AgentList {admin} />
        </section>

        <section aria-labelledby="ag-history">
          <h3 class="label" id="ag-history">{labels.history}</h3>
          <div class="bar">
            <label class="filter">
              {h.filterLabel}
              <select
                data-filter="client"
                value={screen.client ?? ""}
                onchange={(e) => screen.setClient(e.currentTarget.value === "" ? null : e.currentTarget.value)}
              >
                <option value="">{h.filterAll}</option>
                {#each agentNames as name (name)}
                  <option value={name}>{agentLabel(name)}</option>
                {/each}
              </select>
            </label>
            <label class="filter">
              <input
                type="checkbox"
                data-filter="unseen"
                checked={screen.unseenOnly}
                onchange={(e) => screen.setUnseenOnly(e.currentTarget.checked)}
              />
              {h.unseenOnly}
            </label>
            <button
              type="button"
              class="btn mark-all"
              disabled={unseenCount === 0}
              aria-label={h.markAllLabel(unseenCount)}
              onclick={() => void screen.markAllSeen()}
            >
              {h.markAll}
            </button>
          </div>
          {#if screen.result !== null}
            <p class="result" class:warn={screen.result === "mark_failed"} role="status">
              {screen.result === "mark_failed" ? h.markFailed : h.result[screen.result]}
            </p>
          {/if}
          {#if screen.failed}<p class="warn" role="status">{h.loadFailed}</p>{/if}
          {#if !screen.loaded}
            {#if !screen.failed}<p class="muted" role="status">{h.loading}</p>{/if}
          {:else if screen.entries.length === 0}
            <p class="muted" role="status">{screen.unseenOnly ? h.emptyUnseen : h.empty}</p>
          {:else}
            <ul class="changes" aria-label={h.listLabel}>
              {#each screen.entries as e (e.id)}
                {@const unseen = e.actor === "agent" && e.seenAt === null}
                <li class="change" class:reverted={e.revertedAt !== null} data-change={e.id}>
                  <div class="line">
                    {#if unseen}<span class="dot" role="img" aria-label={h.unseen}>◆</span>{/if}
                    <span class="op">{h.operation(e.operation)}</span>
                    {#if e.client !== null}<span class="who">{agentLabel(e.client)}</span>{/if}
                    <time class="when" datetime={new Date(e.createdAt).toISOString()}>{backupTime(e.createdAt)}</time>
                    {#if e.revertedAt !== null}<span class="badge">{h.reverted}</span>{/if}
                  </div>
                  <p class="target">{targetText(e)}</p>
                  {@render summary(e)}
                  <div class="actions">
                    {#if e.actor === "agent"}
                      {#if e.revertedAt === null}
                        <button
                          type="button"
                          class="btn"
                          aria-label={h.revertLabel(h.operation(e.operation))}
                          disabled={screen.flow?.busy === true}
                          onclick={() => void screen.startRevert(e)}
                        >
                          {h.revert}
                        </button>
                      {/if}
                      {#if unseen}
                        <button type="button" class="btn" onclick={() => void screen.markSeen([e.id])}>{h.markSeen}</button>
                      {/if}
                    {:else}
                      <span class="muted">{h.revertRecord}</span>
                    {/if}
                  </div>
                  {@render revertBox(e)}
                </li>
              {/each}
            </ul>
            {#if screen.nextBefore !== null}
              <button type="button" class="btn more" disabled={screen.loadingMore} onclick={() => void screen.loadMore()}>
                {screen.loadingMore ? h.loading : h.loadMore}
              </button>
            {/if}
          {/if}
        </section>
      </div>
    </div>
  </div>
{/if}

<style>
  .ov {
    position: fixed;
    inset: 0;
    z-index: 10;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 8vh 16px 16px;
    background: var(--scrim);
    backdrop-filter: blur(3px);
  }

  /* パネルの形は設定（S-05）・検索パレット（S-04）・持ち越し（S-03）と同じ。変更の一覧が長いので幅を広げる */
  .panel {
    width: min(calc(860 * var(--u)), 100%);
    height: min(calc(660 * var(--u)), 84vh);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: calc(14 * var(--u));
    box-shadow: var(--shadow-panel);
    overflow: hidden;
    outline: none;
  }

  .head {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: calc(14 * var(--u)) calc(24 * var(--u));
    border-bottom: 1px solid var(--rule);
  }

  .title {
    margin: 0;
    font-size: var(--fs-body);
    font-weight: 600;
  }

  kbd {
    font-family: inherit;
    margin-left: 0.4em;
    color: var(--text-muted);
  }

  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: calc(14 * var(--u)) calc(24 * var(--u)) calc(24 * var(--u));
    scrollbar-width: thin;
    scrollbar-color: var(--rule) transparent;
  }

  .label {
    margin: calc(10 * var(--u)) 0 calc(6 * var(--u));
    font-size: var(--fs-label);
    font-weight: 600;
    letter-spacing: 0.14em;
    color: var(--text-muted);
  }

  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(8 * var(--u)) calc(16 * var(--u));
    margin-bottom: calc(10 * var(--u));
    font-size: var(--fs-small);
  }

  .filter {
    display: inline-flex;
    align-items: center;
    gap: calc(6 * var(--u));
  }

  .mark-all {
    margin-left: auto;
  }

  select {
    font: inherit;
    font-size: var(--fs-small);
    color: var(--text);
    padding: calc(4 * var(--u)) calc(8 * var(--u));
    background: var(--field);
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
  }

  .muted {
    margin: calc(6 * var(--u)) 0;
    font-size: var(--fs-small);
    color: var(--text-muted);
  }

  .warn {
    margin: calc(6 * var(--u)) 0;
    font-size: var(--fs-small);
    color: var(--warn);
  }

  .result {
    margin: 0 0 calc(8 * var(--u));
    font-size: var(--fs-small);
  }

  .changes {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .change {
    padding: calc(12 * var(--u)) 0;
    border-bottom: 1px solid var(--rule);
  }

  .change.reverted .op,
  .change.reverted .target,
  .change.reverted .sum {
    color: var(--text-muted);
  }

  .line {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: calc(2 * var(--u)) calc(12 * var(--u));
  }

  .dot {
    color: var(--accent-strong);
  }

  .op {
    font-size: var(--fs-body);
    font-weight: 600;
  }

  .who,
  .when {
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .when {
    font-variant-numeric: tabular-nums;
  }

  .badge {
    font-size: var(--fs-meta);
    padding: 0 calc(8 * var(--u));
    border: 1px solid var(--rule);
    border-radius: calc(10 * var(--u));
    color: var(--text-muted);
  }

  .target {
    margin: calc(4 * var(--u)) 0 0;
    font-size: var(--fs-small);
  }

  .sum {
    margin: calc(4 * var(--u)) 0 0;
    font-size: var(--fs-small);
    color: var(--text-muted);
  }

  .fields {
    list-style: none;
    padding: 0;
  }

  .fname {
    margin-right: 0.4em;
    color: var(--text);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: calc(8 * var(--u));
    margin-top: calc(8 * var(--u));
  }

  .revert {
    margin-top: calc(10 * var(--u));
    padding: calc(10 * var(--u)) calc(12 * var(--u));
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
    background: var(--field);
    font-size: var(--fs-small);
  }

  .revert p {
    margin: 0 0 calc(6 * var(--u));
  }

  .plan {
    border-collapse: collapse;
    margin: 0 0 calc(8 * var(--u));
    font-size: var(--fs-small);
  }

  .plan th,
  .plan td {
    padding: calc(3 * var(--u)) calc(14 * var(--u)) calc(3 * var(--u)) 0;
    text-align: left;
    font-weight: 400;
    vertical-align: top;
  }

  .plan thead th {
    color: var(--text-muted);
    font-size: var(--fs-meta);
  }

  .plan tr.since td {
    color: var(--warn);
  }

  .flag {
    font-size: var(--fs-meta);
  }

  .diff pre {
    margin: 0 0 calc(6 * var(--u));
    padding: calc(6 * var(--u)) calc(8 * var(--u));
    font-family: var(--mono, ui-monospace, monospace);
    font-size: var(--fs-meta);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    border-radius: calc(6 * var(--u));
    background: var(--surface);
  }

  .dlabel {
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .btn {
    font: inherit;
    font-size: var(--fs-small);
    color: var(--text);
    padding: calc(5 * var(--u)) calc(12 * var(--u));
    background: var(--field);
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
    cursor: pointer;
  }

  .btn:hover:not(:disabled) {
    border-color: var(--accent-strong);
  }

  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .btn.primary {
    border-color: var(--accent-strong);
    color: var(--accent-strong);
  }

  .more {
    margin-top: calc(12 * var(--u));
  }
</style>
