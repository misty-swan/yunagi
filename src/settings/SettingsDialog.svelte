<script lang="ts">
  // 設定（S-05。画面設計書 7章）。外枠の上に重ねるパネル（形は検索パレット S-04・持ち越し S-03 と同じ）。
  // 左に区分（General / Display / Operation / Data / Agents）、右にその区分の設定を並べる。
  // 変えた値はすぐ画面に当て、コアの setting に保存する（`SettingsStore.save`）。保存先・バックアップ・
  // 書き出しは DataSection、ショートカット割当は ShortcutsSection、エージェントと権限・ローカル API は
  // AgentsSection。
  //
  // キー：Esc で閉じる（保存先などの確かめる画面を出していれば、先にそれを閉じる）、Mod+, でも閉じる、
  // Tab はこの画面の中だけを回る。開いている間は、ほかの画面のキー（Mod+J・ノートの Mod+Shift+H など）を
  // 受け付けない。区分の一覧は ↑/↓ で移る。
  import { tick } from "svelte";
  import { t } from "../lib/i18n";
  import { imeSupport } from "../lib/ime.svelte";
  import { shellActionOf } from "../lib/keys";
  import {
    FONT_SIZE_MAX,
    FONT_SIZE_MIN,
    minutesOf,
    prefs,
    type Layout,
    type NoteFont,
    type Theme,
  } from "../lib/prefs.svelte";
  import type { DataSettings } from "../lib/settings_data.svelte";
  import { SETTINGS_SECTIONS, type SettingsPanel, type SettingsSection } from "../lib/settings_panel.svelte";
  import type { SettingsStore } from "../lib/settings.svelte";
  import { ShortcutEditor } from "../lib/shortcuts_edit.svelte";
  import type { AgentAdmin } from "../lib/agent_admin.svelte";
  import AgentsSection from "./AgentsSection.svelte";
  import DataSection from "./DataSection.svelte";
  import ShortcutsSection from "./ShortcutsSection.svelte";

  let {
    panel,
    store,
    data,
    admin,
    mac,
    onOpenAgents,
  }: {
    panel: SettingsPanel;
    store: SettingsStore;
    data: DataSettings;
    admin: AgentAdmin;
    mac: boolean;
    /** エージェント変更履歴（S-06）を開く。 */
    onOpenAgents: () => void;
  } = $props();

  const m = $derived(t());
  const s = $derived(m.settings);
  const labels = $derived(m.labels.settings);

  let root = $state<HTMLElement | null>(null);

  // ショートカット割当のキーの取り込み（閉じたら、取り込み中をやめる）
  // svelte-ignore state_referenced_locally
  const editor = new ShortcutEditor(store, mac);
  $effect(() => {
    if (!panel.open) editor.cancel();
  });

  // 開いたら、いまの区分へフォーカスを置く。閉じたら、開く直前にフォーカスのあった所へ戻す
  let wasOpen = false;
  $effect(() => {
    if (panel.open) {
      wasOpen = true;
      void tick().then(() => root?.querySelector<HTMLElement>(`[data-section="${panel.section}"]`)?.focus());
      return;
    }
    if (!wasOpen) return;
    wasOpen = false;
    data.stop();
    data.reset();
    const back = panel.returnFocus;
    panel.returnFocus = null;
    const a = document.activeElement;
    if (back instanceof HTMLElement && back.isConnected && (a === null || a === document.body)) back.focus();
  });

  // 操作区分を開いたとき（Vimモードが有効なら）、IME の切替ができる環境かを調べる（制約の表示。操作仕様書 4.4 の 4）
  $effect(() => {
    if (panel.open && panel.section === "operation" && prefs.vimMode) void imeSupport.refresh();
  });

  // データ区分を開いたら、保存先とバックアップの一覧を読む
  $effect(() => {
    if (panel.open && panel.section === "data") void data.load();
  });

  // エージェント区分を開くたびに、エージェントの一覧とローカル API の状態を読む。区分を離れる・閉じるときは、
  // 確かめている途中の操作と知らせを捨てる
  $effect(() => {
    if (panel.open && panel.section === "agents") {
      const release = admin.retain();
      void admin.load();
      void admin.loadApi();
      return release;
    }
  });

  const error = $derived.by(() => {
    const e = store.error;
    if (e === null) return null;
    return e.kind === "invalid_argument" ? s.invalid : s.saveFailed;
  });

  function focusables(): HTMLElement[] {
    if (root === null) return [];
    return [
      ...root.querySelectorAll<HTMLElement>(
        "button, input, select, textarea, a[href], [tabindex]:not([tabindex='-1'])",
      ),
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
      if (data.flow !== null) data.cancel();
      else if (admin.ask !== null) admin.cancelAsk();
      else if (admin.removing !== null) admin.cancelRemove();
      else panel.close();
      return;
    }
    const action = shellActionOf(e, mac);
    if (action === null) return;
    // 骨組みの操作はしない。Mod+, はもう一度押すと閉じる
    e.preventDefault();
    if (action === "settings" && !e.repeat) panel.close();
  }

  // 背後にフォーカスがあるときのキーも、ほかの画面の操作にしない（覆いの外へフォーカスが落ちたとき）。
  // Esc は閉じ、ほかは区分へ戻す
  function onWindowKeydown(e: KeyboardEvent) {
    if (!panel.open || root === null) return;
    if (e.target instanceof Node && root.contains(e.target)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === "Escape") panel.close();
    else focusables()[0]?.focus();
  }

  function onNavKeydown(e: KeyboardEvent, section: SettingsSection) {
    const delta = e.key === "ArrowDown" ? 1 : e.key === "ArrowUp" ? -1 : 0;
    if (delta === 0 || e.ctrlKey || e.metaKey || e.altKey) return;
    e.preventDefault();
    const i = SETTINGS_SECTIONS.indexOf(section);
    const next = SETTINGS_SECTIONS[(i + delta + SETTINGS_SECTIONS.length) % SETTINGS_SECTIONS.length];
    panel.section = next;
    void tick().then(() => root?.querySelector<HTMLElement>(`[data-section="${next}"]`)?.focus());
  }

  function pickTime(e: Event): string {
    return (e.currentTarget as HTMLInputElement).value;
  }

  /** 空や読めない値は変えない（時刻の欄を消したとき）。 */
  function saveTime(e: Event, field: "dayBoundary" | "meterStart" | "meterEnd") {
    const v = pickTime(e);
    const el = e.currentTarget as HTMLInputElement;
    const minutes = minutesOf(v);
    // 始業が終業より前でない組は、コアに送らず、描きもしない（凪メーターが一瞬おかしくならないように）
    const start = field === "meterStart" ? minutes : minutesOf(prefs.meterStart);
    const end = field === "meterEnd" ? minutes : minutesOf(prefs.meterEnd);
    const meterBad = field !== "dayBoundary" && start !== null && end !== null && start >= end;
    if (minutes === null) {
      el.value = prefs[field];
    } else if (meterBad) {
      el.value = prefs[field];
      store.error = { kind: "invalid_argument", reason: "meter_hours" };
    } else {
      void store.save({ [field]: v });
    }
  }

  function saveThreshold(e: Event) {
    const el = e.currentTarget as HTMLInputElement;
    const n = Number(el.value);
    if (Number.isInteger(n) && n >= 1) void store.save({ carryoverReviewThreshold: n });
    else el.value = String(prefs.reviewThreshold);
  }
</script>

<svelte:window onkeydowncapture={onWindowKeydown} />

{#snippet row(label: string, id: string, control: import("svelte").Snippet, note?: string)}
  <div class="row">
    <label class="name" for={id}>{label}</label>
    <div class="control">{@render control()}</div>
    {#if note}<p class="note">{note}</p>{/if}
  </div>
{/snippet}

{#if panel.open}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="ov" onkeydown={onKeydown} onmousedown={(e) => e.target === e.currentTarget && panel.close()}>
    <div class="panel" role="dialog" aria-modal="true" aria-label={s.label} data-settings bind:this={root}>
      <nav class="nav" aria-label={s.sectionsLabel}>
        <h2 class="title">{s.label}</h2>
        <ul>
          {#each SETTINGS_SECTIONS as section (section)}
            <li>
              <button
                type="button"
                class="nav-btn label"
                data-section={section}
                aria-current={panel.section === section ? "true" : undefined}
                tabindex={panel.section === section ? 0 : -1}
                onclick={() => (panel.section = section)}
                onkeydown={(e) => onNavKeydown(e, section)}
              >
                {labels[section]}
              </button>
            </li>
          {/each}
        </ul>
        <button type="button" class="close" onclick={() => panel.close()}>{s.close} <kbd>Esc</kbd></button>
      </nav>

      <div class="content" data-pane={panel.section}>
        {#if error !== null}<p class="err" role="alert">{error}</p>{/if}

        {#if panel.section === "general"}
          {@const g = s.general}
          <h3 class="label">{labels.general}</h3>
          {#snippet dayBoundary()}
            <input id="set-day-boundary" class="fc-field fc-time" type="time" value={prefs.dayBoundary} onchange={(e) => saveTime(e, "dayBoundary")} />
          {/snippet}
          {@render row(g.dayBoundary, "set-day-boundary", dayBoundary, g.dayBoundaryNote)}
          {#snippet meter()}
            <label class="inline">
              <span>{g.meterStart}</span>
              <input id="set-meter-start" class="fc-field fc-time" type="time" value={prefs.meterStart} onchange={(e) => saveTime(e, "meterStart")} />
            </label>
            <label class="inline">
              <span>{g.meterEnd}</span>
              <input id="set-meter-end" class="fc-field fc-time" type="time" value={prefs.meterEnd} onchange={(e) => saveTime(e, "meterEnd")} />
            </label>
          {/snippet}
          <div class="row" role="group" aria-labelledby="set-meter-name">
            <span class="name" id="set-meter-name">{g.meterHours}</span>
            <div class="control">{@render meter()}</div>
            <p class="note">{g.meterNote}</p>
          </div>
          {#snippet language()}
            <select id="set-lang" class="fc-field fc-select" value={prefs.lang} onchange={(e) => void store.save({ locale: e.currentTarget.value as "ja" | "en" })}>
              <option value="ja">{g.languageOptions.ja}</option>
              <option value="en">{g.languageOptions.en}</option>
            </select>
          {/snippet}
          {@render row(g.language, "set-lang", language)}
          {#snippet update()}
            <input
              id="set-update"
              class="fc-check"
              type="checkbox"
              checked={prefs.updateCheck}
              onchange={(e) => void store.save({ updateCheck: e.currentTarget.checked })}
            />
          {/snippet}
          {@render row(g.update, "set-update", update, g.updateNote)}
        {:else if panel.section === "display"}
          {@const d = s.display}
          <h3 class="label">{labels.display}</h3>
          {#snippet theme()}
            <select id="set-theme" class="fc-field fc-select" value={prefs.theme} onchange={(e) => void store.save({ theme: e.currentTarget.value as Theme })}>
              {#each ["system", "light", "dark"] as const as v (v)}
                <option value={v}>{d.themeOptions[v]}</option>
              {/each}
            </select>
          {/snippet}
          {@render row(d.theme, "set-theme", theme)}
          {#snippet fontSize()}
            <input
              id="set-font-size"
              class="fc-range"
              type="range"
              min={FONT_SIZE_MIN}
              max={FONT_SIZE_MAX}
              step="1"
              value={prefs.fontSize}
              aria-valuetext={d.fontSizeValue(prefs.fontSize)}
              onchange={(e) => void store.save({ fontSize: Number(e.currentTarget.value) })}
            />
            <output for="set-font-size">{d.fontSizeValue(prefs.fontSize)}</output>
          {/snippet}
          {@render row(d.fontSize, "set-font-size", fontSize)}
          {#snippet noteFont()}
            <select id="set-note-font" class="fc-field fc-select" value={prefs.noteFont} onchange={(e) => void store.save({ noteFont: e.currentTarget.value as NoteFont })}>
              {#each ["body", "mono"] as const as v (v)}
                <option value={v}>{d.noteFontOptions[v]}</option>
              {/each}
            </select>
          {/snippet}
          {@render row(d.noteFont, "set-note-font", noteFont)}
          {#snippet layout()}
            <select id="set-layout" class="fc-field fc-select" value={prefs.layout} onchange={(e) => void store.save({ layout: e.currentTarget.value as Layout })}>
              {#each ["A", "B", "C"] as const as v (v)}
                <option value={v}>{d.layoutOptions[v]}</option>
              {/each}
            </select>
          {/snippet}
          {@render row(d.layout, "set-layout", layout, d.layoutNote)}
        {:else if panel.section === "operation"}
          {@const o = s.operation}
          <h3 class="label">{labels.operation}</h3>
          {#snippet threshold()}
            <input id="set-threshold" class="fc-field fc-number" type="number" min="1" step="1" value={prefs.reviewThreshold} onchange={saveThreshold} />
          {/snippet}
          {@render row(o.reviewThreshold, "set-threshold", threshold, o.reviewThresholdNote)}
          {#snippet vim()}
            <input
              id="set-vim"
              class="fc-check"
              type="checkbox"
              checked={prefs.vimMode}
              onchange={(e) => void store.save({ vimMode: e.currentTarget.checked })}
            />
          {/snippet}
          {@render row(o.vim, "set-vim", vim, o.vimNote)}
          {#if prefs.vimMode && !mac}<p class="note vim-alt">{o.vimAltNote}</p>{/if}
          {#if prefs.vimMode}
            {#snippet imeRestore()}
              <input
                id="set-vim-ime-restore"
                class="fc-check"
                type="checkbox"
                checked={prefs.vimImeRestore}
                onchange={(e) => void store.save({ vimImeRestore: e.currentTarget.checked })}
              />
            {/snippet}
            {@render row(o.vimImeRestore, "set-vim-ime-restore", imeRestore, o.vimImeRestoreNote)}
            {#if imeSupport.unavailable}<p class="note ime-unavailable">{o.imeUnavailable}</p>{/if}
          {/if}
          <h3 class="label sub">{o.shortcuts}</h3>
          <ShortcutsSection {editor} {mac} />
        {:else if panel.section === "data"}
          <h3 class="label">{labels.data}</h3>
          <DataSection {data} />
        {:else}
          <h3 class="label">{labels.agents}</h3>
          <AgentsSection {admin} {onOpenAgents} />
        {/if}
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

  /* パネルの形は検索パレット（S-04）・持ち越し（S-03）と同じ */
  .panel {
    width: min(calc(780 * var(--u)), 100%);
    height: min(calc(620 * var(--u)), 84vh);
    display: flex;
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: calc(14 * var(--u));
    box-shadow: var(--shadow-panel);
    overflow: hidden;
  }

  .nav {
    flex: none;
    width: calc(180 * var(--u));
    display: flex;
    flex-direction: column;
    padding: calc(18 * var(--u)) calc(10 * var(--u));
    border-right: 1px solid var(--rule);
  }

  .title {
    margin: 0 0 calc(12 * var(--u)) calc(10 * var(--u));
    font-size: var(--fs-body);
    font-weight: 600;
  }

  .nav ul {
    list-style: none;
    margin: 0;
    padding: 0;
    flex: 1;
  }

  .label {
    font-size: var(--fs-label);
    font-weight: 600;
    letter-spacing: 0.14em;
    color: var(--text-muted);
  }

  .nav-btn {
    display: block;
    width: 100%;
    text-align: left;
    padding: calc(8 * var(--u)) calc(10 * var(--u));
    border: 0;
    border-radius: calc(8 * var(--u));
    background: transparent;
    font-family: inherit;
    cursor: pointer;
  }

  .nav-btn:hover {
    background: var(--sel);
  }

  .nav-btn[aria-current="true"] {
    color: var(--accent-strong);
    background: var(--sel);
    box-shadow: inset 2px 0 0 var(--accent-strong);
  }

  .close {
    font: inherit;
    font-size: var(--fs-meta);
    color: var(--text-muted);
    background: transparent;
    border: 0;
    border-radius: calc(8 * var(--u));
    padding: calc(6 * var(--u)) calc(10 * var(--u));
    text-align: left;
    cursor: pointer;
  }

  .close:hover {
    background: var(--sel);
  }

  kbd {
    font-family: inherit;
    margin-left: 0.4em;
  }

  .content {
    flex: 1;
    min-width: 0;
    overflow: auto;
    padding: calc(18 * var(--u)) calc(26 * var(--u)) calc(26 * var(--u));
    scrollbar-width: thin;
    scrollbar-color: var(--rule) transparent;
  }

  .content :global(h3.label) {
    margin: 0 0 calc(10 * var(--u));
  }

  .panel :global(.err) {
    margin: 0 0 calc(10 * var(--u));
    color: var(--warn);
    font-size: var(--fs-small);
  }

  .panel :global(.row) {
    display: grid;
    grid-template-columns: calc(190 * var(--u)) 1fr;
    gap: calc(4 * var(--u)) calc(16 * var(--u));
    align-items: center;
    padding: calc(12 * var(--u)) 0;
    border-bottom: 1px solid var(--rule);
  }

  .panel :global(.name) {
    font-size: var(--fs-small);
  }

  .panel :global(.control) {
    display: flex;
    align-items: center;
    gap: calc(12 * var(--u));
    min-width: 0;
  }

  .panel :global(.note) {
    grid-column: 2;
    margin: 0;
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .panel :global(.placeholder .name) {
    color: var(--text-muted);
  }

  .content :global(h3.label.sub) {
    margin-top: calc(22 * var(--u));
  }

  .vim-alt {
    margin: calc(6 * var(--u)) 0 0;
    font-size: var(--fs-meta);
    color: var(--text-muted);
  }

  .inline {
    display: inline-flex;
    align-items: center;
    gap: calc(6 * var(--u));
    font-size: var(--fs-small);
    color: var(--text-muted);
  }

  /* 選択欄・入力欄・チェックボックス・スライダーの見た目は src/styles/forms.css（06 11章） */
  output {
    font-size: var(--fs-small);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .panel :global(.btn) {
    font: inherit;
    font-size: var(--fs-small);
    color: var(--text);
    padding: calc(6 * var(--u)) calc(14 * var(--u));
    background: var(--field);
    border: 1px solid var(--ring);
    border-radius: calc(8 * var(--u));
    cursor: pointer;
  }

  .panel :global(.btn:hover:not(:disabled)) {
    border-color: var(--accent-strong);
  }

  .panel :global(.btn:active:not(:disabled)) {
    background: color-mix(in srgb, var(--accent-strong) 22%, transparent);
    border-color: var(--accent-strong);
  }

  .panel :global(.btn:focus-visible) {
    border-radius: calc(8 * var(--u));
  }

  .panel :global(.btn:disabled) {
    opacity: 0.5;
    cursor: default;
  }

  .panel :global(.btn.primary) {
    border-color: var(--accent-strong);
    color: var(--accent-strong);
  }

  @media (max-width: 640px) {
    .panel {
      flex-direction: column;
    }

    .nav {
      width: auto;
      border-right: 0;
      border-bottom: 1px solid var(--rule);
    }

    .nav ul {
      display: flex;
      flex: none;
      flex-wrap: wrap;
    }

    .row {
      grid-template-columns: 1fr;
    }

    .note {
      grid-column: 1;
    }
  }
</style>
