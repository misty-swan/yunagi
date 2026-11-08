<script lang="ts">
  // エージェントの一覧（画面設計書 7章・8章）：識別子・最後に接続した時刻・権限の切り替え。設定の
  // エージェント区分と変更履歴 S-06 が同じ一覧を使う（状態と操作は AgentAdmin）。
  // 権限を入れるときは、足す権限で何ができるかを書いた確かめる文をその行の下に出す。外すときはすぐ行う。
  // 登録を消す（`showRemove`。設定だけ）のも、確かめる文を出してから行う。
  import { tick } from "svelte";
  import { AGENT_SCOPES, CLI_AGENT, UNKNOWN_AGENT, type AgentAdmin } from "../lib/agent_admin.svelte";
  import type { AgentRow, AgentScope } from "../lib/bindings";
  import { backupTime } from "../lib/backups";
  import { agentLabel, t } from "../lib/i18n";
  import type { AdminNotice } from "../lib/agent_admin.svelte";

  let { admin, showRemove = false }: { admin: AgentAdmin; showRemove?: boolean } = $props();

  const m = $derived(t());
  const a = $derived(m.agents);

  let root = $state<HTMLElement | null>(null);

  const notices: Record<AdminNotice, "scopesChanged" | "scopesFailed" | "unknownWrite" | "removeInUse" | "removeFailed" | "notFound"> = {
    scopes_changed: "scopesChanged",
    scopes_failed: "scopesFailed",
    unknown_write: "unknownWrite",
    remove_in_use: "removeInUse",
    remove_failed: "removeFailed",
    not_found: "notFound",
  };

  /** 識別子と、製品の名前が違うときは両方（`Claude Code`・`claude-code`）。 */
  function title(row: AgentRow): { label: string; id: string | null } {
    const label = agentLabel(row.name);
    return { label, id: label === row.name ? null : row.name };
  }

  function onScopeChange(e: Event, row: AgentRow, scope: AgentScope) {
    const el = e.currentTarget as HTMLInputElement;
    const on = el.checked;
    // 確かめる文を出す間、権限はまだ変わっていない。見た目はコアが答えた権限のままにする
    el.checked = !on;
    admin.toggle(row.name, scope, on);
  }

  // 確かめる文を出したら、その中のボタンへフォーカスを移す
  const asking = $derived(admin.ask?.client ?? admin.removing);
  $effect(() => {
    if (asking === null) return;
    // 初めのフォーカスは「やめる」（許可する・消すではなく）。キーボードの二度押しで確定しないように
    void tick().then(() => root?.querySelector<HTMLElement>(".confirm [data-initial]")?.focus());
  });
</script>

<div class="agents" bind:this={root}>
  {#if admin.notice !== null}
    <p class="err" role="alert">{a.notices[notices[admin.notice]]}</p>
  {/if}
  {#if admin.loadFailed && admin.agents === null}
    <p class="note" role="status">{a.loadFailed}</p>
  {:else if admin.agents !== null}
    <ul>
      {#each admin.agents as row (row.id)}
        {@const names = title(row)}
        <li class="agent" data-agent={row.name}>
          <div class="who">
            <span class="aname">{names.label}</span>
            {#if names.id !== null}<code class="aid">{names.id}</code>{/if}
            <span class="seen">
              {row.lastSeenAt === null ? a.neverSeen : a.lastSeen(backupTime(row.lastSeenAt))}
            </span>
            {#if showRemove}
              {#if row.removable}
                <button
                  type="button"
                  class="btn"
                  disabled={admin.busy}
                  aria-label={a.removeLabel(row.name)}
                  onclick={() => admin.askRemove(row.name)}
                >
                  {a.remove}
                </button>
              {:else if row.name !== CLI_AGENT && row.name !== UNKNOWN_AGENT}
                <span class="inuse">{a.inUse}</span>
              {/if}
            {/if}
          </div>
          {#if row.name === CLI_AGENT || row.name === UNKNOWN_AGENT}
            <p class="fixed">{a.fixed[row.name]}</p>
          {/if}
          <div class="scopes" role="group" aria-label={a.scopesOf(row.name)}>
            {#each AGENT_SCOPES as scope (scope)}
              {@const locked = row.name === UNKNOWN_AGENT && scope !== "read"}
              <label class="scope" class:locked title={locked ? a.unknownReadOnly : undefined}>
                <input
                  class="fc-check"
                  type="checkbox"
                  data-scope={scope}
                  checked={row.scopes.includes(scope)}
                  disabled={admin.busy || (locked && !row.scopes.includes(scope))}
                  onchange={(e) => onScopeChange(e, row, scope)}
                />
                {a.scopes[scope]}
              </label>
            {/each}
          </div>
          {#if admin.ask !== null && admin.ask.client === row.name}
            {@const ask = admin.ask}
            <div class="confirm" role="group" aria-label={a.scopes[ask.scope]}>
              <p>{a.grantConfirm(names.label, a.scopes[ask.scope], a.scopeHelp[ask.scope])}</p>
              <div class="actions">
                <button type="button" class="btn primary" onclick={() => void admin.confirmAsk()}>{a.grantYes}</button>
                <button type="button" class="btn" data-initial onclick={() => admin.cancelAsk()}>{a.grantNo}</button>
              </div>
            </div>
          {/if}
          {#if admin.removing === row.name}
            <div class="confirm" role="group" aria-label={a.removeLabel(row.name)}>
              <p>{a.removeConfirm(names.label)}</p>
              <div class="actions">
                <button type="button" class="btn primary" onclick={() => void admin.confirmRemove()}>{a.removeYes}</button>
                <button type="button" class="btn" data-initial onclick={() => admin.cancelRemove()}>{a.removeNo}</button>
              </div>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .err {
    margin: 0 0 calc(10 * var(--u));
    color: var(--warn);
    font-size: var(--fs-small);
  }

  .note {
    margin: 0;
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .agent {
    padding: calc(12 * var(--u)) 0;
    border-bottom: 1px solid var(--rule);
  }

  .who {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: calc(4 * var(--u)) calc(12 * var(--u));
  }

  .aname {
    font-size: var(--fs-body);
    font-weight: 600;
  }

  .aid {
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .seen,
  .inuse {
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .who .btn {
    margin-left: auto;
  }

  .fixed {
    margin: calc(4 * var(--u)) 0 0;
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .scopes {
    display: flex;
    flex-wrap: wrap;
    gap: calc(6 * var(--u)) calc(16 * var(--u));
    margin-top: calc(8 * var(--u));
  }

  .scope {
    display: inline-flex;
    align-items: center;
    gap: calc(6 * var(--u));
    font-size: var(--fs-small);
  }

  .scope.locked {
    color: var(--text-muted);
  }

  .confirm {
    margin-top: calc(10 * var(--u));
    padding: calc(10 * var(--u)) calc(12 * var(--u));
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
    background: var(--field);
    font-size: var(--fs-small);
  }

  .confirm p {
    margin: 0 0 calc(8 * var(--u));
  }

  .actions {
    display: flex;
    gap: calc(8 * var(--u));
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
</style>
