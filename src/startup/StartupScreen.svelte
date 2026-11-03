<script lang="ts">
  // 起動時の画面（画面設計書 10.1）。データベースを開くまでの間、窓全体に出す。
  // - 開いている途中：静かに「データを開いています」。1 秒を超えたら、ロックを待っていることを添える。
  // - 見つからない：探した場所と、待ってやり直す・保存先を選び直す・バックアップから戻す・新しく作るの
  //   4つの案内（アーキテクチャ設計書 6.3）。
  // - 開けなかった：理由ごとに、何が起きたか・データが無事か・次にすること（やり直す、旧版・新しい版の
  //   入れ直し、バックアップから戻す）・問い合わせ先・詳しい情報（ログ用の文）を出す。
  // - バックアップから戻した後：退避した前のデータベースの場所を示し、「続ける」で整理モードへ進む。
  import { getVersion } from "@tauri-apps/api/app";
  import type { BackupEntry, Startup, StartupFailure } from "../lib/bindings";
  import { shortDate } from "../lib/dates";
  import { t } from "../lib/i18n";
  import type { StartupActions } from "../lib/startup.svelte";
  import { ISSUES_URL, SUPPORT_MAIL, currentOs, installCommand } from "../lib/support";
  import Logo from "../shell/Logo.svelte";

  let { startup, actions }: { startup: Startup | null; actions: StartupActions } = $props();

  /** 「開いています」に、ロックを待っていることを添えるまでの時間。 */
  const SLOW_MS = 1000;

  const m = $derived(t().startup);
  const os = currentOs();
  const phase = $derived(startup?.state ?? "opening");
  const reason = $derived(startup?.state === "failed" ? startup.reason : null);
  /** バックアップから戻せる状態（見つからない、壊れていて開けない）。 */
  const canRestore = $derived(phase === "not_found" || reason === "corrupted");
  /** 旧版の入れ直しを案内する理由。 */
  const REINSTALL: StartupFailure[] = ["migration_failed", "corrupted"];
  /** 問い合わせ先を出す理由（利用者の操作だけでは直らないことがあるもの）。 */
  const CONTACT: StartupFailure[] = [
    "default_dir",
    "lock",
    "config",
    "migration_failed",
    "migration_restore_failed",
    "unsupported_schema",
    "corrupted",
    "storage",
  ];
  /** やり直しを出す理由。 */
  const RETRY: StartupFailure[] = ["default_dir", "lock_busy", "lock", "config", "already_exists", "storage"];

  let version = $state("");
  $effect(() => {
    getVersion()
      .then((v) => (version = `v${v}`))
      .catch(() => {});
  });

  let slow = $state(false);
  $effect(() => {
    if (phase !== "opening") {
      slow = false;
      return;
    }
    const timer = setTimeout(() => (slow = true), SLOW_MS);
    return () => clearTimeout(timer);
  });

  // 状態が変わるたびに、バックアップの一覧を読み直す（戻せる状態のときだけ）
  const listKey = $derived(canRestore ? `${phase}:${reason ?? ""}` : null);
  $effect(() => {
    if (listKey !== null) void actions.loadBackups();
  });

  let selected = $state<string | null>(null);
  // 一覧が変わったら、選んでいたものが無ければ先頭（いちばん新しいもの）を選ぶ
  $effect(() => {
    const list = actions.backups ?? [];
    if (!list.some((b) => b.path === selected)) selected = list[0]?.path ?? null;
  });
  let dir = $state("");
  let confirmCreate = $state(false);

  // 状態が変わったら見出しへフォーカスを移す（読み上げで、何が起きたかが伝わるように）
  let heading = $state<HTMLElement | null>(null);
  $effect(() => {
    void phase;
    void reason;
    void actions.restored;
    heading?.focus();
  });

  const busy = $derived(actions.busy !== null);

  /**
   * 操作の欄の外（見出しの下）に出す失敗：始める前に止まったときは、状態が変わってその欄が消えている
   * ことがあり、前のデータベースを戻せなかったときは、問い合わせ先と一緒に示すため。
   */
  const topError = $derived.by((): string | null => {
    const e = actions.error?.error;
    if (e?.kind === "not_ready") return m.notAttempted;
    if (e?.kind === "put_back_failed") return m.putBackFailed(e.setAside);
    return null;
  });

  function errorText(): string | null {
    const e = actions.error;
    if (e === null || topError !== null) return null;
    if (e.error.kind === "rejected") return m.rejected[e.error.reason];
    return {
      retry: m.retryFailed,
      create: m.createFailed,
      choose: m.chooseFailed,
      restore: m.restoreFailed,
    }[e.action];
  }
  const error = $derived(errorText());

  function pad(n: number) {
    return String(n).padStart(2, "0");
  }

  /** 作った時刻（端末のタイムゾーンで `2026.11.03 14:05`）。 */
  function createdAt(ms: number): string {
    const d = new Date(ms);
    return `${d.getFullYear()}.${pad(d.getMonth() + 1)}.${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  }

  function kindLabel(b: BackupEntry): string {
    switch (b.kind.type) {
      case "daily":
        return m.backupDaily(shortDate(b.kind.date));
      case "pre_migration":
        return m.backupPreMigration(b.kind.from, b.kind.to);
      case "pre_restore":
        return m.backupPreRestore;
    }
  }

  function onChoose(e: SubmitEvent) {
    e.preventDefault();
    if (dir.trim() !== "") void actions.choose(dir);
  }
</script>

{#snippet errorLine(action: string)}
  {#if error !== null && actions.error?.action === action}
    <p class="error" role="alert">{error}</p>
  {/if}
{/snippet}

{#snippet topErrorBlock()}
  {#if topError !== null}
    <p class="error" role="alert">{topError}</p>
    {#if actions.error?.error.kind === "put_back_failed"}{@render contact()}{/if}
  {/if}
{/snippet}

{#snippet path(label: string, value: string | null)}
  {#if value !== null}
    <div class="path">
      <span class="path-label">{label}</span>
      <code>{value}</code>
    </div>
  {/if}
{/snippet}

{#snippet restoreSection(body: string)}
  <section class="option" aria-labelledby="startup-restore">
    <h2 id="startup-restore">{m.restoreTitle}</h2>
    <p>{body}</p>
    {#if actions.backupsFailed}
      <p class="muted">{m.backupsFailed}</p>
    {:else if actions.backups !== null && actions.backups.length === 0}
      <p class="muted">{m.noBackups}</p>
    {:else if actions.backups !== null}
      <fieldset class="backups">
        <legend>{m.backupsLabel}</legend>
        {#each actions.backups as b (b.path)}
          <label class="backup">
            <input type="radio" name="startup-backup" value={b.path} bind:group={selected} disabled={busy} />
            <span class="kind">{kindLabel(b)}</span>
            <span class="time">{createdAt(b.createdAt)}</span>
          </label>
        {/each}
      </fieldset>
      <button
        type="button"
        class="btn"
        disabled={busy || selected === null}
        onclick={() => selected !== null && void actions.restore(selected)}
      >
        {actions.busy === "restore" ? m.working : m.restore}
      </button>
      {@render errorLine("restore")}
    {/if}
  </section>
{/snippet}

{#snippet contact()}
  <section class="option" aria-labelledby="startup-contact">
    <h2 id="startup-contact">{m.contactTitle}</h2>
    <p>{m.contactBody}</p>
    {@render path(m.contactIssues, ISSUES_URL)}
    {@render path(m.contactMail, SUPPORT_MAIL)}
  </section>
{/snippet}

{#snippet install(title: string, body: string, version?: string)}
  <section class="option" aria-label={title}>
    <h2>{title}</h2>
    <p>{body}</p>
    <pre class="command"><code>{installCommand(os, version)}</code></pre>
  </section>
{/snippet}

<div class="startup" data-startup={phase} data-reason={reason ?? ""}>
  <div class="column">
    <div class="brand"><Logo /></div>

    {#if actions.restored !== null}
      <h1 tabindex="-1" bind:this={heading}>{m.restoredTitle}</h1>
      <p>{m.restoredBody}</p>
      {#if actions.restored.setAside !== null}
        <p>{m.restoredSetAside(actions.restored.setAside)}</p>
      {/if}
      <button type="button" class="btn primary" onclick={() => actions.acknowledge()}>{m.continue}</button>
    {:else if phase === "opening" || phase === "ready"}
      <h1 tabindex="-1" bind:this={heading} class="quiet" aria-live="polite">{m.opening}</h1>
      {#if slow}<p class="muted">{m.openingWait}</p>{/if}
    {:else if startup?.state === "not_found"}
      <h1 tabindex="-1" bind:this={heading}>{m.notFoundTitle}</h1>
      {@render topErrorBlock()}
      {@render path(m.searched, startup.path)}
      {#if startup.configuredIn !== null}<p class="muted">{m.configuredIn(startup.configuredIn)}</p>{/if}
      <p>{startup.dataDirExists ? m.maybeSyncing : m.noFolder}</p>

      <section class="option" aria-labelledby="startup-wait">
        <h2 id="startup-wait">{m.waitTitle}</h2>
        <p>{m.waitBody}</p>
        <button type="button" class="btn" disabled={busy} onclick={() => void actions.retry()}>
          {actions.busy === "retry" ? m.working : m.retry}
        </button>
        {@render errorLine("retry")}
      </section>

      <section class="option" aria-labelledby="startup-choose">
        <h2 id="startup-choose">{m.chooseTitle}</h2>
        <p>{m.chooseBody}</p>
        <form class="choose" onsubmit={onChoose}>
          <label>
            <span class="path-label">{m.chooseLabel}</span>
            <input type="text" bind:value={dir} spellcheck="false" autocomplete="off" disabled={busy} />
          </label>
          <button type="submit" class="btn" disabled={busy || dir.trim() === ""}>
            {actions.busy === "choose" ? m.working : m.choose}
          </button>
        </form>
        {@render errorLine("choose")}
      </section>

      {@render restoreSection(m.restoreBody)}

      <section class="option" aria-labelledby="startup-create">
        <h2 id="startup-create">{m.createTitle}</h2>
        <p>{m.createBody}</p>
        {#if confirmCreate}
          <p>{m.createConfirm}</p>
          <div class="row">
            <button type="button" class="btn" disabled={busy} onclick={() => void actions.create()}>
              {actions.busy === "create" ? m.working : m.createYes}
            </button>
            <button type="button" class="btn ghost" disabled={busy} onclick={() => (confirmCreate = false)}>
              {m.createNo}
            </button>
          </div>
        {:else}
          <button type="button" class="btn" disabled={busy} onclick={() => (confirmCreate = true)}>{m.create}</button>
        {/if}
        {@render errorLine("create")}
      </section>
    {:else if startup?.state === "failed"}
      <h1 tabindex="-1" bind:this={heading}>{m.failedTitle[startup.reason]}</h1>
      {@render topErrorBlock()}
      <p>{m.failedBody[startup.reason]}</p>
      {@render path(m.databaseAt, startup.database)}
      {@render path(m.backupAt, startup.backup)}
      {@render path(m.configAt, startup.config)}

      {#if RETRY.includes(startup.reason)}
        <div class="row">
          <button type="button" class="btn" disabled={busy} onclick={() => void actions.retry()}>
            {actions.busy === "retry" ? m.working : m.retry}
          </button>
        </div>
        {@render errorLine("retry")}
      {/if}

      {#if startup.reason === "corrupted"}
        {@render restoreSection(m.restoreDamagedBody)}
      {/if}
      {#if REINSTALL.includes(startup.reason)}
        {@render install(
          m.reinstallTitle,
          startup.reason === "corrupted" ? m.reinstallDamagedBody(version) : m.reinstallBody(version),
          m.versionPlaceholder,
        )}
      {/if}
      {#if startup.reason === "unsupported_schema"}
        {@render install(m.upgradeTitle, m.upgradeBody(version))}
      {/if}

      {#if CONTACT.includes(startup.reason) && actions.error?.error.kind !== "put_back_failed"}
        {@render contact()}
      {/if}

      <details class="detail">
        <summary>{m.detailTitle}</summary>
        <pre><code>{[version, startup.detail].filter((s) => s !== "").join("\n")}</code></pre>
      </details>
    {/if}
  </div>
</div>

<style>
  .startup {
    min-height: 100%;
    display: flex;
    justify-content: center;
    padding: calc(64 * var(--u)) var(--pad);
    background: var(--bg);
    color: var(--text);
  }

  .column {
    width: 100%;
    max-width: calc(600 * var(--u));
    display: flex;
    flex-direction: column;
    gap: calc(14 * var(--u));
  }

  .brand {
    margin-bottom: calc(10 * var(--u));
  }

  h1 {
    margin: 0;
    font-size: calc(20 * var(--u));
    font-weight: 600;
    line-height: 1.4;
  }

  h1.quiet {
    font-size: var(--fs-body);
    font-weight: 500;
    color: var(--text-muted);
  }

  h2 {
    margin: 0;
    font-size: var(--fs-body);
    font-weight: 600;
  }

  p {
    margin: 0;
  }

  .muted {
    color: var(--text-muted);
    font-size: var(--fs-small);
  }

  .option {
    display: flex;
    flex-direction: column;
    gap: calc(10 * var(--u));
    align-items: flex-start;
    padding: calc(16 * var(--u)) calc(18 * var(--u));
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: calc(12 * var(--u));
  }

  .option p {
    font-size: var(--fs-small);
    color: var(--text-muted);
  }

  .path {
    display: flex;
    flex-direction: column;
    gap: calc(2 * var(--u));
    font-size: var(--fs-small);
  }

  .path-label {
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  code,
  pre {
    font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
    font-size: var(--fs-meta);
    overflow-wrap: anywhere;
    white-space: pre-wrap;
    user-select: text;
  }

  .command {
    margin: 0;
    width: 100%;
    padding: calc(8 * var(--u)) calc(10 * var(--u));
    background: var(--bg);
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
  }

  .row {
    display: flex;
    gap: calc(10 * var(--u));
  }

  .btn {
    padding: calc(6 * var(--u)) calc(14 * var(--u));
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
    background: var(--surface);
    font-size: var(--fs-small);
    font-weight: 500;
    cursor: pointer;
  }

  .btn:hover:not(:disabled) {
    background: var(--hover);
  }

  .btn:disabled {
    color: var(--text-muted);
    cursor: default;
  }

  .btn.primary {
    border-color: var(--accent-strong);
    color: var(--accent-strong);
  }

  .btn.ghost {
    border-color: transparent;
    background: transparent;
  }

  .choose {
    width: 100%;
    display: flex;
    gap: calc(10 * var(--u));
    align-items: flex-end;
  }

  .choose label {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: calc(4 * var(--u));
  }

  .choose input {
    font: inherit;
    font-size: var(--fs-small);
    color: var(--text);
    padding: calc(6 * var(--u)) calc(10 * var(--u));
    background: var(--field);
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
  }

  .backups {
    width: 100%;
    margin: 0;
    padding: 0;
    border: 0;
    display: flex;
    flex-direction: column;
    gap: calc(4 * var(--u));
  }

  .backups legend {
    padding: 0;
    margin-bottom: calc(4 * var(--u));
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .backup {
    display: flex;
    gap: calc(10 * var(--u));
    align-items: center;
    font-size: var(--fs-small);
  }

  .backup .time {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .error {
    color: var(--warn);
    font-size: var(--fs-small);
  }

  .option .error {
    color: var(--warn);
  }

  .detail summary {
    cursor: pointer;
    font-size: var(--fs-small);
    color: var(--text-muted);
  }

  .detail pre {
    margin: calc(8 * var(--u)) 0 0;
    color: var(--text-muted);
  }
</style>
