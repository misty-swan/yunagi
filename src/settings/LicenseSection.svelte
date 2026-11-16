<script lang="ts">
  // 設定のライセンス区分（画面設計書 7章「ライセンス区分と『このアプリについて』」）：サポーターライセンスの
  // キーの入力と「確認する」、確認済みの表示と「キーを消す」。状態と操作は LicenseStore
  // （src/lib/license.svelte.ts）。キーが無くても、確認できなくても、機能は変わらない。
  import { tick } from "svelte";
  import { t } from "../lib/i18n";
  import { verifiedDate, type LicenseStore } from "../lib/license.svelte";

  let { license }: { license: LicenseStore } = $props();

  const s = $derived(t().settings.license);
  const verified = $derived(license.verified);
  const unavailable = $derived(license.state?.state === "unavailable");

  let root = $state<HTMLElement | null>(null);

  // 確かめる文の初めのフォーカスは「やめる」（エージェント区分と同じ。二度押しで消さないように）
  $effect(() => {
    if (!license.confirmingRemove) return;
    void tick().then(() => root?.querySelector<HTMLElement>(".confirm [data-initial]")?.focus());
  });

  function onSubmit(e: SubmitEvent) {
    e.preventDefault();
    void license.verify();
  }
</script>

<div class="license" bind:this={root}>
  {#if license.loadFailed && license.state === null}
    <p class="err" role="alert">{s.loadFailed}</p>
  {:else if license.state === null}
    <!-- 読み込み中は何も出さない -->
  {:else if verified !== null}
    <div class="row">
      <span class="name">{s.key}</span>
      <div class="control">
        <span class="status" data-license="verified">{s.status(verified.keyTail, verifiedDate(verified.verifiedAt))}</span>
      </div>
    </div>
    {#if license.confirmingRemove}
      <div class="confirm" role="group" aria-label={s.remove}>
        <p>{s.removeConfirm}</p>
        <div class="actions">
          <button type="button" class="btn primary" onclick={() => void license.confirmRemove()}>{s.removeYes}</button>
          <button type="button" class="btn" data-initial onclick={() => license.cancelRemove()}>{s.removeNo}</button>
        </div>
      </div>
    {:else}
      <div class="actions">
        <button type="button" class="btn" onclick={() => license.askRemove()}>{s.remove}</button>
      </div>
    {/if}
  {:else if unavailable}
    <p class="note" data-license="unavailable">{s.unavailable}</p>
  {:else}
    <form class="row" onsubmit={onSubmit}>
      <label class="name" for="set-license-key">{s.key}</label>
      <div class="control">
        <input
          id="set-license-key"
          class="fc-field key"
          type="text"
          autocomplete="off"
          autocapitalize="off"
          spellcheck="false"
          bind:value={license.input}
          disabled={license.verifying}
        />
        <button type="submit" class="btn primary" disabled={license.verifying || license.input.trim() === ""}>{s.verify}</button>
      </div>
      <p class="note">{s.networkNote}</p>
    </form>
  {/if}
  <p class="result" role="status" aria-live="polite">
    {#if license.verifying && verified === null}
      {s.verifying}
    {:else if license.notice !== null}
      {s.notices[license.notice]}
    {/if}
  </p>
</div>

<style>
  .key {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono, monospace);
  }

  .status {
    font-size: var(--fs-small);
  }

  .actions {
    display: flex;
    gap: calc(8 * var(--u));
    margin-top: calc(10 * var(--u));
  }

  .confirm {
    margin-top: calc(10 * var(--u));
    font-size: var(--fs-small);
  }

  .confirm p {
    margin: 0;
  }

  .result {
    margin: calc(10 * var(--u)) 0 0;
    font-size: var(--fs-small);
    min-height: 1em;
  }

  .err {
    color: var(--warn);
  }
</style>
