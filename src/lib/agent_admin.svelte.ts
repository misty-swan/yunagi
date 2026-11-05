// エージェントの一覧と権限の切り替え・登録の整理・ローカル API の状態（画面設計書 7章・8章）。設定の
// エージェント区分（src/settings/）と変更履歴 S-06（src/agents/）が同じ一覧と操作を使う。見た目は
// src/agents/AgentList.svelte。
//
// - 権限を足すときは、足す権限で何ができるかを書いた確かめる文を出してから行う（`ask`。確認を経ずに
//   書き込みの権限を与えないため。MCP・CLI 仕様書 2章）。外すときは、確かめずにすぐ行う（権限を狭める操作の
//   ため）。コアには、見せた権限を `expect` に付けて送る（確かめている間に別の所で変わっていれば、書かずに
//   `conflict`。一覧を読み直して伝える）。
// - 登録を消すのは、登録の数の上限（64）に達したときの整理（MCP・CLI 仕様書 9章）。変更履歴にも項目にも
//   指されていない登録だけ消せる。消す前に確かめる文（`askRemove`）を出す。
// - データが変わったとき（`changed`。CLI・MCP が初めての識別子を登録したときも）は、一覧を読み直す。

import type { UnlistenFn } from "@tauri-apps/api/event";
import type { AgentRow, AgentScope, ApiStatus } from "./bindings";
import { core, coreEvents, errorOf, keepListening } from "./core";

/** 権限を並べる順（MCP・CLI 仕様書 2章の表の順）。 */
export const AGENT_SCOPES: readonly AgentScope[] = ["read", "todo", "blocker", "append", "note_edit"];

/** 識別子のない接続の登録の名前（`read` より多くの権限は足せない）。 */
export const UNKNOWN_AGENT = "unknown";

/** 許可する前に確かめている権限。`shown` は確かめる画面で見せた、そのエージェントのいまの権限。 */
export interface ScopeAsk {
  client: string;
  scope: AgentScope;
  shown: AgentScope[];
}

/** 操作の結果の知らせ（次の操作を始めると消す）。 */
export type AdminNotice =
  | "scopes_changed"
  | "scopes_failed"
  | "unknown_write"
  | "remove_in_use"
  | "remove_failed"
  | "not_found";

export class AgentAdmin {
  /** エージェントの一覧。読む前と、読めなかったときは `null`。 */
  agents = $state<AgentRow[] | null>(null);
  loadFailed = $state(false);
  api = $state<ApiStatus | null>(null);
  apiFailed = $state(false);
  /** 許可する前に確かめている権限。 */
  ask = $state<ScopeAsk | null>(null);
  /** 消す前に確かめている登録の名前。 */
  removing = $state<string | null>(null);
  /** 書き込みを行っている間。 */
  busy = $state(false);
  notice = $state<AdminNotice | null>(null);

  #loads = 0;
  #stopped = false;
  readonly #unlisten: UnlistenFn[] = [];

  /** データの変わり方（changed）を聞き始める。読み込み済みなら、一覧を読み直す。 */
  start() {
    void (async () => {
      const u = await keepListening(
        () =>
          coreEvents.onChanged(() => {
            if (this.agents !== null) void this.load();
          }),
        () => this.#stopped,
      );
      if (u !== null) this.#unlisten.push(u);
    })();
  }

  stop() {
    this.#stopped = true;
    this.#loads++;
    for (const u of this.#unlisten.splice(0)) u();
  }

  /** 一覧を読む。 */
  async load() {
    const n = ++this.#loads;
    try {
      const rows = await core.listAgents();
      if (n !== this.#loads || this.#stopped) return;
      this.agents = rows;
      this.loadFailed = false;
    } catch {
      if (n !== this.#loads || this.#stopped) return;
      this.loadFailed = true;
    }
  }

  /** ローカル API の状態を読む（設定のエージェント区分を開くたびに）。 */
  async loadApi() {
    try {
      const status = await core.apiStatus();
      if (this.#stopped) return;
      this.api = status;
      this.apiFailed = false;
    } catch {
      if (this.#stopped) return;
      this.apiFailed = true;
    }
  }

  /** 開き直したときに、確かめている途中の操作と知らせを捨てる。 */
  reset() {
    this.ask = null;
    this.removing = null;
    this.notice = null;
  }

  scopesOf(client: string): AgentScope[] {
    return this.agents?.find((a) => a.name === client)?.scopes ?? [];
  }

  /** 権限 `scope` を入れる／切る。入れるときは確かめる文を出す。切るときはすぐ行う。 */
  toggle(client: string, scope: AgentScope, on: boolean) {
    if (this.busy) return;
    this.notice = null;
    this.removing = null;
    const shown = this.scopesOf(client);
    if (on) {
      if (client === UNKNOWN_AGENT && scope !== "read") {
        this.notice = "unknown_write";
        return;
      }
      this.ask = { client, scope, shown };
      return;
    }
    this.ask = null;
    void this.#apply(client, [], [scope], shown);
  }

  cancelAsk() {
    this.ask = null;
  }

  /** 確かめた権限を許可する。 */
  async confirmAsk() {
    const a = this.ask;
    if (a === null || this.busy) return;
    this.ask = null;
    await this.#apply(a.client, [a.scope], [], a.shown);
  }

  askRemove(client: string) {
    if (this.busy) return;
    this.notice = null;
    this.ask = null;
    this.removing = client;
  }

  cancelRemove() {
    this.removing = null;
  }

  /** 確かめた登録を消す。 */
  async confirmRemove() {
    const client = this.removing;
    if (client === null || this.busy) return;
    this.removing = null;
    this.busy = true;
    this.notice = null;
    try {
      await core.removeAgent(client);
    } catch (e) {
      const err = errorOf(e);
      if (err.kind === "conflict" && err.reason === "client_in_use") this.notice = "remove_in_use";
      else if (err.kind === "not_found") this.notice = "not_found";
      else this.notice = "remove_failed";
    } finally {
      this.busy = false;
    }
    await this.load();
  }

  async #apply(client: string, allow: AgentScope[], deny: AgentScope[], expect: AgentScope[]) {
    this.busy = true;
    this.notice = null;
    try {
      await core.setAgentScopes(client, allow, deny, expect);
    } catch (e) {
      const err = errorOf(e);
      if (err.kind === "conflict" && err.reason === "scopes_changed") this.notice = "scopes_changed";
      else if (err.kind === "invalid_argument" && err.reason === "unknown_write") this.notice = "unknown_write";
      else if (err.kind === "not_found") this.notice = "not_found";
      else this.notice = "scopes_failed";
    } finally {
      this.busy = false;
    }
    await this.load();
  }
}
