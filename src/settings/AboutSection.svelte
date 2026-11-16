<script lang="ts">
  // 設定の「このアプリについて」区分（画面設計書 7章「ライセンス区分と『このアプリについて』」）：版、本体の
  // ライセンス、サードパーティライセンス。キーが確認できているときだけ、サポーターの表示（ありがとうの
  // 一文）を添える。確認できていなければ何も出さない（購入の案内も出さない）。
  import { t } from "../lib/i18n";
  import type { AboutStore, LicenseStore } from "../lib/license.svelte";

  let { about, license }: { about: AboutStore; license: LicenseStore } = $props();

  const s = $derived(t().settings.about);
</script>

<div class="row">
  <span class="name">{s.version}</span>
  <div class="control"><span data-about="version">{about.version === "" ? "" : `v${about.version}`}</span></div>
</div>
<div class="row">
  <span class="name">{s.license}</span>
  <div class="control"><span>{s.licenseText}</span></div>
</div>
<div class="row">
  <span class="name">{s.thirdParty}</span>
  <div class="control">
    <button type="button" class="btn" aria-expanded={about.open} aria-controls="set-third-party" onclick={() => void about.toggle()}>
      {about.open ? s.hideThirdParty : s.showThirdParty}
    </button>
  </div>
</div>
{#if about.open}
  <div id="set-third-party">
    {#if about.thirdParty}
      <!-- 長い表記を、キーボードでもスクロールできるようにフォーカスを受ける -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <pre class="notices" tabindex="0" aria-label={s.thirdParty}>{about.thirdParty}</pre>
    {:else if about.thirdPartyFailed}
      <p class="note missing" role="alert">{s.thirdPartyFailed}</p>
    {:else}
      <p class="note missing">{s.thirdPartyMissing}</p>
    {/if}
  </div>
{/if}
{#if license.verified !== null}
  <p class="supporter" data-about="supporter">{s.supporter}</p>
{/if}

<style>
  .notices {
    margin: calc(10 * var(--u)) 0 0;
    max-height: calc(260 * var(--u));
    overflow: auto;
    padding: calc(10 * var(--u));
    border: 1px solid var(--rule);
    border-radius: calc(8 * var(--u));
    font-size: var(--fs-meta);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .missing {
    margin: calc(10 * var(--u)) 0 0;
  }

  .supporter {
    margin: calc(18 * var(--u)) 0 0;
    font-size: var(--fs-small);
    color: var(--accent-strong);
  }
</style>
