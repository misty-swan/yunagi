<script lang="ts">
  // 設定のエージェント区分（画面設計書 7章）：接続したエージェントと権限（切り替え・登録の整理）、
  // ローカル API の状態、変更履歴 S-06 を開く。一覧と操作は AgentAdmin（src/lib/agent_admin.svelte.ts）。
  import type { AgentAdmin } from "../lib/agent_admin.svelte";
  import { t } from "../lib/i18n";
  import AgentList from "../agents/AgentList.svelte";

  let { admin, onOpenAgents }: { admin: AgentAdmin; onOpenAgents: () => void } = $props();

  const m = $derived(t());
  const s = $derived(m.settings.agents);
  const a = $derived(m.agents);
  const labels = $derived(m.labels.agentsScreen);

  const apiText = $derived.by((): { text: string; warn: boolean } | null => {
    if (admin.apiFailed) return { text: a.api.failed, warn: true };
    const api = admin.api;
    if (api === null) return null;
    switch (api.state) {
      case "listening":
        return {
          text: api.connections > 0 ? `${a.api.running} · ${a.api.connections(api.connections)}` : a.api.running,
          warn: false,
        };
      case "stopped":
        return { text: a.api.stopped, warn: false };
      case "unavailable":
        return { text: a.api.unavailable[api.cause], warn: true };
    }
  });
</script>

<section class="group" aria-labelledby="set-agents-list">
  <h4 class="sub" id="set-agents-list">{s.connected}</h4>
  <AgentList {admin} showRemove />
</section>

<section class="group" aria-labelledby="set-agents-api">
  <h4 class="sub" id="set-agents-api">{s.localApi}</h4>
  <div class="row">
    <span class="name">{s.apiState}</span>
    <div class="control">
      <span class="api" class:warn={apiText?.warn} data-api={admin.api?.state ?? ""}>{apiText?.text ?? ""}</span>
    </div>
    <p class="note">{a.api.note}</p>
  </div>
</section>

<section class="group" aria-labelledby="set-agents-history">
  <h4 class="sub" id="set-agents-history">{s.history}</h4>
  <div class="row">
    <span class="name">{labels.title}</span>
    <div class="control">
      <button type="button" class="btn" onclick={onOpenAgents}>{s.openHistory}</button>
    </div>
  </div>
</section>

<style>
  .group {
    margin-bottom: calc(22 * var(--u));
  }

  .sub {
    margin: 0 0 calc(4 * var(--u));
    font-size: var(--fs-body);
    font-weight: 600;
  }

  .api.warn {
    color: var(--warn);
  }
</style>
