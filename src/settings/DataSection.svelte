<script lang="ts">
  // 設定のデータ区分（画面設計書 7章）：保存先（変更・既存のフォルダの選び直し・クラウド同期フォルダの
  // 警告）、バックアップの一覧と復元、Markdown 書き出し。操作は DataSettings（src/lib/settings_data.svelte.ts）。
  // 保存先の変更・選び直し・復元は、実行する前に確かめる文を出す（実行しても警告は止めない）。
  import { backupKindLabel, backupTime } from "../lib/backups";
  import type { CallError } from "../lib/core";
  import { t } from "../lib/i18n";
  import type { DataResult, DataSettings } from "../lib/settings_data.svelte";

  let { data }: { data: DataSettings } = $props();

  const m = $derived(t());
  const d = $derived(m.settings.data);
  const labels = $derived(m.labels.settings);
  const busy = $derived(data.busy !== null);

  /** 失敗の文（コアのエラーの種類から決める。内部の文は出さない）。 */
  function failure(r: Extract<DataResult, { kind: "failed" }>): string {
    const e: CallError = r.error;
    if (e.kind === "rejected") {
      // 保存先の変更では、データベースが無いのは「いまの保存先」。選び直しでは「選んだフォルダ」
      return d.rejected[e.reason];
    }
    switch (r.action) {
      case "move":
        return d.moveFailed;
      case "switch":
        return d.switchFailed;
      case "restore":
        return d.restoreFailed;
      case "export":
        if (e.kind === "export_failed") {
          return e.replaced > 0
            ? `${d.exportWriteFailed} ${d.exportPartial(e.replaced)}`
            : d.exportWriteFailed;
        }
        return d.exportReadFailed;
    }
  }

  const resultText = $derived.by((): { text: string; warn: boolean } | null => {
    const r = data.result;
    if (r === null) return null;
    switch (r.kind) {
      case "moved":
        return {
          text: r.leftovers.length > 0 ? `${d.moved} ${d.leftovers(r.leftovers.length)}` : d.moved,
          warn: r.leftovers.length > 0,
        };
      case "exported":
        return { text: d.exported(r.count, r.dir), warn: false };
      case "nothing":
        return { text: d.exportNothing, warn: false };
      case "unsaved":
        return { text: d.unsaved, warn: true };
      case "failed":
        return { text: failure(r), warn: true };
    }
  });

  const flowCloud = $derived(
    data.flow !== null && data.flow.kind !== "restore" && data.flow.cloud !== null ? d.cloud[data.flow.cloud] : null,
  );
</script>

<section class="group" aria-labelledby="set-data-location">
  <h4 class="sub" id="set-data-location">{d.locationTitle}</h4>
  <div class="row">
    <span class="name">{d.location}</span>
    <div class="control"><code class="path">{data.dataDir ?? d.unknownLocation}</code></div>
  </div>
  <div class="row">
    <span class="name"></span>
    <div class="control">
      <button type="button" class="btn" disabled={busy} onclick={() => void data.chooseMove(d.movePick)}>{d.move}</button>
      <button type="button" class="btn" disabled={busy} onclick={() => void data.chooseSwitch(d.switchPick)}>{d.switchTo}</button>
    </div>
    <p class="note">{d.syncNote}</p>
  </div>

  {#if data.flow !== null && data.flow.kind !== "restore"}
    {@const flow = data.flow}
    <div class="confirm" role="group" aria-label={flow.kind === "move" ? d.move : d.switchTo}>
      <p>{flow.kind === "move" ? d.moveConfirm(flow.dir) : d.switchConfirm(flow.dir)}</p>
      {#if flowCloud !== null}<p class="warn" role="alert">{d.cloudWarning(flowCloud)}</p>{/if}
      <div class="actions">
        <button type="button" class="btn primary" disabled={busy} onclick={() => void data.confirm()}>
          {busy ? d.working : flow.kind === "move" ? d.moveYes : d.switchYes}
        </button>
        <button type="button" class="btn" disabled={busy} onclick={() => data.cancel()}>{d.no}</button>
      </div>
    </div>
  {/if}
</section>

<section class="group" aria-labelledby="set-data-backups">
  <h4 class="sub label" id="set-data-backups">{labels.backups}</h4>
  {#if data.backupsFailed}
    <p class="note solo">{d.backupsFailed}</p>
  {:else if data.backups !== null && data.backups.length === 0}
    <p class="note solo">{d.backupsEmpty}</p>
  {:else if data.backups !== null}
    <ul class="backups">
      {#each data.backups as b (b.path)}
        {@const label = backupKindLabel(b, m.startup)}
        <li class="backup">
          <span class="kind">{label}</span>
          <span class="time">{backupTime(b.createdAt)}</span>
          <button
            type="button"
            class="btn"
            disabled={busy}
            aria-label={d.restoreButton(label)}
            onclick={() => data.askRestore(b)}
          >
            {d.restoreYes}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  <p class="note solo">{d.backupsNote}</p>

  {#if data.flow !== null && data.flow.kind === "restore"}
    {@const b = data.flow.backup}
    <div class="confirm" role="group" aria-label={d.restoreYes}>
      <p>{d.restoreConfirm(backupKindLabel(b, m.startup), backupTime(b.createdAt))}</p>
      <div class="actions">
        <button type="button" class="btn primary" disabled={busy} onclick={() => void data.confirm()}>
          {busy ? d.working : d.restoreYes}
        </button>
        <button type="button" class="btn" disabled={busy} onclick={() => data.cancel()}>{d.no}</button>
      </div>
    </div>
  {/if}
</section>

<section class="group" aria-labelledby="set-data-export">
  <h4 class="sub label" id="set-data-export">{labels.export}</h4>
  <p class="note solo title">{d.exportTitle}</p>
  <div class="row">
    <span class="name">{d.exportFrom} / {d.exportTo}</span>
    <div class="control">
      <label class="inline">
        <span>{d.exportFrom}</span>
        <input type="date" bind:value={data.exportFrom} disabled={busy} />
      </label>
      <label class="inline">
        <span>{d.exportTo}</span>
        <input type="date" bind:value={data.exportTo} disabled={busy} />
      </label>
    </div>
  </div>
  <div class="row">
    <span class="name">{d.exportDir}</span>
    <div class="control">
      <code class="path">{data.exportDir ?? d.exportNoDir}</code>
      <button type="button" class="btn" disabled={busy} onclick={() => void data.pickExportDir(d.exportPickTitle)}>
        {d.exportPick}
      </button>
    </div>
  </div>
  <div class="row">
    <span class="name"></span>
    <div class="control">
      <button
        type="button"
        class="btn primary"
        disabled={busy || data.exportDir === null || data.exportFrom === "" || data.exportTo === ""}
        onclick={() => void data.runExport()}
      >
        {data.busy === "export" ? d.working : d.exportRun}
      </button>
    </div>
  </div>
</section>

<!-- 操作の結果（読み上げにも伝える） -->
<p class="result" class:warn={resultText?.warn === true} class:none={resultText === null} role="status">
  {resultText?.text ?? ""}
</p>

<style>
  .group {
    margin-bottom: calc(18 * var(--u));
  }

  .sub {
    margin: calc(14 * var(--u)) 0 calc(4 * var(--u));
    font-size: var(--fs-small);
    font-weight: 600;
  }

  .sub.label {
    font-size: var(--fs-label);
    letter-spacing: 0.14em;
    color: var(--text-muted);
  }

  .path {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: var(--fs-meta);
    overflow-wrap: anywhere;
    user-select: text;
  }

  .confirm {
    margin: calc(8 * var(--u)) 0;
    padding: calc(12 * var(--u)) calc(14 * var(--u));
    border: 1px solid var(--rule);
    border-radius: calc(10 * var(--u));
    background: var(--field);
    font-size: var(--fs-small);
  }

  .confirm p {
    margin: 0 0 calc(8 * var(--u));
  }

  .warn {
    color: var(--warn);
  }

  .actions {
    display: flex;
    gap: calc(10 * var(--u));
  }

  .backups {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .backup {
    display: flex;
    align-items: center;
    gap: calc(14 * var(--u));
    min-height: var(--row-h);
    border-bottom: 1px solid var(--rule);
    font-size: var(--fs-small);
  }

  .kind {
    flex: 1;
    min-width: 0;
  }

  .time {
    flex: none;
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    font-size: var(--fs-meta);
  }

  .solo {
    margin: calc(6 * var(--u)) 0;
  }

  .title {
    color: var(--text);
    font-size: var(--fs-small);
  }

  .inline {
    display: inline-flex;
    align-items: center;
    gap: calc(6 * var(--u));
    font-size: var(--fs-small);
    color: var(--text-muted);
  }

  .result {
    margin: 0;
    font-size: var(--fs-small);
    color: var(--text-muted);
  }

  .result.none {
    display: none;
  }

  .result.warn {
    color: var(--warn);
  }
</style>
