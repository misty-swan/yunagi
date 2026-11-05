// エージェント変更履歴（S-06。画面設計書 8章）の状態：開いているか、変更の一覧、絞り込み、取り消しの確認、
// 確認済みの印。見た目は src/agents/Agents.svelte。権限の切り替えは AgentAdmin（agent_admin.svelte.ts）。
//
// - 開いたのは、サイドバーの Agents・上部のバーの「◆ 件数」・検索パレットのコマンド・設定のエージェント区分の
//   「変更履歴を開く」のどれか（どれも App の `openScreen` を通る）。閉じたら、開く直前にフォーカスのあった所へ戻る。
// - 一覧は新しい順に 50 件ずつ読む（`nextBefore` がある間は「さらに読み込む」）。開いただけでは確認済みの
//   印を付けない（上部のバーの件数と、整理モードの行頭の ◆ が、見ていない間に消えないように）。行ごとの
//   「確認済みにする」と、表示している記録すべての「確認済みにする」で付ける（見せた記録の ID だけを送る）。
// - 取り消しは、その行の下に確かめる文を出してから行う（`flow`）。確かめる前に、まだ保存していない入力を送る
//   （差分と確認の目印が、送る前の古い本文から作られないように）。人が記録のあとに変えた欄があれば、
//   通常の「取り消す」は出さず、見せた値を上書きする「上書きして取り消す」だけを出す。確かめた後に内容が
//   変わっていれば、書かずに確かめ直す。
// - データが変わったとき（changed）は、開いている間だけ、読み込み済みの件数（最大 1000 件）を読み直す。読み直した一覧に無くなった記録の取り消しの確かめる文は閉じる。

import type { UnlistenFn } from "@tauri-apps/api/event";
import type { ChangeEntry, Id, RevertPlan } from "./bindings";
import { core, coreEvents, errorOf, keepListening, LISTEN_RETRY_MS, type CallError } from "./core";
import type { RevertProblem } from "./i18n_agents";

/** 一覧を 1 回に読む件数。 */
export const PAGE_SIZE = 50;
/** 読み直すときの件数の上限。 */
export const REFRESH_MAX = 1000;
/** コアが 1 回に返せる件数の上限。 */
const PAGE_MAX = 200;
/** 読み込みに失敗したとき、読み直すまでの時間。 */
export const AGENTS_RETRY_MS = LISTEN_RETRY_MS;

export interface AgentsDeps {
  /** まだ保存していない入力を送り終えるまで待つ。 */
  flush: () => Promise<unknown>;
}

/** 取り消せない理由（`detail` は文に入れる値）。 */
export interface Problem {
  kind: RevertProblem;
  date?: string;
}

/** 取り消しの確認。 */
export interface RevertFlow {
  entry: ChangeEntry;
  /** 取り消す内容。読むまでと、読めなかったときは `null`。 */
  plan: RevertPlan | null;
  /** 取り消せない理由（あれば `plan` の代わりに出す）。 */
  problem: Problem | null;
  /** 確かめた後に内容が変わっていた（確かめ直した）。 */
  changedAfterPreview: boolean;
  /** 取り消しを行っている間。 */
  busy: boolean;
}

/** 取り消しの結果の知らせ。 */
export type AgentsResult = "reverted" | "marked" | "already_reverted" | "mark_failed" | "client_gone";

/** コアのエラーを、取り消せない理由にする。 */
export function problemOf(e: CallError): Problem {
  if (e.kind === "conflict") {
    if (e.reason === "user_editing") return { kind: "user_editing" };
    if (e.reason === "carried_over") return { kind: "carried_over", date: e.carriedToDate };
    if (e.reason === "save_failing") return { kind: "save_failing" };
  }
  if (e.kind === "not_found") return { kind: "not_found" };
  if (e.kind === "invalid_argument") {
    if (e.reason === "dropped") return { kind: "dropped" };
    if (e.reason === "not_agent_change") return { kind: "not_agent_change" };
    if (e.reason === "unsupported_record") return { kind: "unsupported" };
  }
  return { kind: "failed" };
}

export class AgentsScreen {
  open = $state(false);
  /** 変更の一覧（新しい順）。読むまでは空。 */
  entries = $state<ChangeEntry[]>([]);
  /** 続きがあるとき、次に読む `before`。 */
  nextBefore = $state<Id | null>(null);
  loaded = $state(false);
  loadingMore = $state(false);
  /** 読み込みに失敗した（読み直している）。 */
  failed = $state(false);
  /** 「さらに読み込む」に失敗した（ボタンを残して、押し直せる）。 */
  moreFailed = $state(false);
  /** 絞り込み：エージェントの識別子（`null` はすべて）。 */
  client = $state<string | null>(null);
  unseenOnly = $state(false);
  flow = $state<RevertFlow | null>(null);
  result = $state<AgentsResult | null>(null);
  /** 増えたら、パネルへフォーカスを置く。 */
  focusSeq = $state(0);
  /** 開く直前にフォーカスのあった所（閉じたら戻す）。 */
  returnFocus: Element | null = null;
  /** 設定のエージェント区分から開いたとき、利用者が閉じたら行う処理（設定を開き直す。画面設計書 10章）。 */
  onBack: (() => void) | null = null;

  readonly #deps: AgentsDeps;
  #loads = 0;
  #retry: ReturnType<typeof setTimeout> | undefined;
  #stopped = false;
  readonly #unlisten: UnlistenFn[] = [];

  constructor(deps: AgentsDeps) {
    this.#deps = deps;
  }

  /** データの変わり方（changed）を聞き始める。 */
  start() {
    void (async () => {
      const u = await keepListening(
        () => coreEvents.onChanged(() => this.#refresh()),
        () => this.#stopped,
      );
      if (u !== null) this.#unlisten.push(u);
    })();
  }

  stop() {
    this.#stopped = true;
    this.#loads++;
    clearTimeout(this.#retry);
    for (const u of this.#unlisten.splice(0)) u();
  }

  /** 表示中の未確認の記録（エージェントの変更で、確認済みでないもの）。 */
  get unseen(): ChangeEntry[] {
    return this.entries.filter((e) => e.actor === "agent" && e.seenAt === null);
  }

  /** 開く。開いていれば、パネルへフォーカスを置き直すだけにする。 */
  show() {
    if (this.open) {
      this.focusSeq++;
      return;
    }
    this.returnFocus = document.activeElement;
    this.open = true;
    this.entries = [];
    this.nextBefore = null;
    this.loaded = false;
    this.failed = false;
    this.moreFailed = false;
    this.loadingMore = false;
    this.client = null;
    this.unseenOnly = false;
    this.flow = null;
    this.result = null;
    this.focusSeq++;
    void this.#load(PAGE_SIZE);
  }

  /** 閉じる（別の画面へ移るとき、持ち越しが出たとき。戻る処理 `onBack` は捨てる）。 */
  close() {
    this.open = false;
    this.flow = null;
    this.onBack = null;
    this.#loads++;
    clearTimeout(this.#retry);
  }

  /** 利用者が閉じる（Esc・「閉じる」・外側を押す）。設定から開いていれば、設定に戻る。 */
  dismiss() {
    const back = this.onBack;
    this.close();
    back?.();
  }

  setClient(client: string | null) {
    this.client = client;
    this.flow = null;
    this.result = null;
    this.loaded = false;
    this.moreFailed = false;
    void this.#load(PAGE_SIZE);
  }

  setUnseenOnly(on: boolean) {
    this.unseenOnly = on;
    this.flow = null;
    this.result = null;
    this.loaded = false;
    this.moreFailed = false;
    void this.#load(PAGE_SIZE);
  }

  #refresh() {
    if (!this.open) return;
    const n = Math.min(Math.max(this.entries.length, PAGE_SIZE), REFRESH_MAX);
    void this.#load(n);
  }

  async #load(limit: number) {
    const n = ++this.#loads;
    clearTimeout(this.#retry);
    try {
      // コアは 1 回に 200 件まで。読み込んでいた件数まで、続けて読む（読み直しで一覧が短くならないように）
      let want = Math.min(limit, PAGE_MAX);
      let page = await core.listChanges(this.client, this.unseenOnly, null, want);
      const all = [...page.changes];
      // 200 件を超えて読み込んでいたときだけ、続けて読む
      while (page.nextBefore !== null && page.changes.length === want && all.length < limit) {
        want = Math.min(limit - all.length, PAGE_MAX);
        page = await core.listChanges(this.client, this.unseenOnly, page.nextBefore, want);
        all.push(...page.changes);
      }
      if (n !== this.#loads || !this.open) return;
      this.entries = all;
      this.nextBefore = page.nextBefore;
      // 読み直した一覧に無くなった記録の確かめる文は閉じる（見えない確かめる文が残らないように）
      if (this.flow !== null && !this.flow.busy && !all.some((e) => e.id === this.flow!.entry.id)) this.flow = null;
      this.loaded = true;
      this.failed = false;
      this.moreFailed = false;
    } catch (e) {
      if (n !== this.#loads || !this.open) return;
      // 絞り込んだエージェントの登録が消えていた：絞り込みを外して読み直す
      if (errorOf(e).kind === "not_found" && this.client !== null) {
        this.#dropClient(limit);
        return;
      }
      this.failed = true;
      this.#retry = setTimeout(() => void this.#load(limit), AGENTS_RETRY_MS);
    }
  }

  /** 絞り込んだエージェントの登録が消えていた：絞り込みを外して読み直し、外したことを知らせる。 */
  #dropClient(limit = PAGE_SIZE) {
    this.client = null;
    this.result = "client_gone";
    void this.#load(limit);
  }

  /** 続きを読む。 */
  async loadMore() {
    const before = this.nextBefore;
    if (before === null || this.loadingMore) return;
    const n = this.#loads;
    this.loadingMore = true;
    try {
      const page = await core.listChanges(this.client, this.unseenOnly, before, PAGE_SIZE);
      if (n !== this.#loads || !this.open) return;
      const known = new Set(this.entries.map((e) => e.id));
      this.entries = [...this.entries, ...page.changes.filter((e) => !known.has(e.id))];
      this.nextBefore = page.nextBefore;
      this.moreFailed = false;
    } catch (e) {
      if (n !== this.#loads || !this.open) return;
      // 絞り込んだ登録が消えていた：絞り込みを外して先頭から読み直す。それ以外は「さらに読み込む」を残して、
      // 押し直せるようにする（一覧の読み込みの失敗にはしない）
      if (errorOf(e).kind === "not_found" && this.client !== null) this.#dropClient();
      else this.moreFailed = true;
    } finally {
      this.loadingMore = false;
    }
  }

  // ---- 確認済み ----

  /** 記録 `ids` を確認済みにする（見せた記録の ID だけを送る）。 */
  async markSeen(ids: Id[]) {
    if (ids.length === 0) return;
    this.result = null;
    try {
      const marked = await core.markChangesSeen(ids);
      // 付けた記録があれば、件数と一覧はコアの changed で読み直す。無いとき（すでに確認済み）は届かないので読み直す
      if (marked === 0) this.#refresh();
    } catch {
      this.result = "mark_failed";
      this.#refresh();
    }
  }

  markAllSeen() {
    return this.markSeen(this.unseen.map((e) => e.id));
  }

  // ---- 取り消し ----

  /** 取り消しの確認を始める：入力を送り終えてから、取り消す内容を読む。 */
  async startRevert(entry: ChangeEntry) {
    this.result = null;
    this.flow = { entry, plan: null, problem: null, changedAfterPreview: false, busy: false };
    await this.#preview(entry, false);
  }

  async #preview(entry: ChangeEntry, changedAfterPreview: boolean) {
    try {
      await this.#deps.flush();
    } catch {
      // 送れなかった（保存に失敗している）。取り消しは、コアが保存の失敗中として断る（理由を出す）
    }
    try {
      const plan = await core.previewRevert(entry.id);
      if (this.flow?.entry.id !== entry.id) return;
      this.flow = { entry, plan, problem: null, changedAfterPreview, busy: false };
    } catch (e) {
      if (this.flow?.entry.id !== entry.id) return;
      this.flow = { entry, plan: null, problem: problemOf(errorOf(e)), changedAfterPreview: false, busy: false };
      if (this.flow.problem?.kind === "not_found") this.#refresh();
    }
  }

  cancelRevert() {
    this.flow = null;
  }

  /**
   * 確かめた取り消しを行う。`force` は人があとから変えた欄を、見せた値で上書きする（見せた内容に
   * 変わった欄があるときだけ）。
   */
  async confirmRevert(force: boolean) {
    const f = this.flow;
    if (f === null || f.plan === null || f.busy) return;
    const { entry, plan } = f;
    this.flow = { ...f, busy: true };
    try {
      const r = await core.revertChange(entry.id, force, plan.token);
      if (this.flow?.entry.id !== entry.id) return;
      this.flow = null;
      this.result = r.updated ? "reverted" : r.marked ? "marked" : "already_reverted";
      // 印を付けたときは、コアの changed で読み直す。何も変えていないときは changed が届かないので読み直す
      if (!r.marked) this.#refresh();
    } catch (e) {
      if (this.flow?.entry.id !== entry.id) return;
      const err = errorOf(e);
      if (err.kind === "conflict" && err.reason === "changed") {
        // 確かめた後に内容が変わった：書かずに、確かめ直す
        await this.#preview(entry, true);
        return;
      }
      this.flow = { entry, plan: null, problem: problemOf(err), changedAfterPreview: false, busy: false };
      if (this.flow.problem?.kind === "not_found") this.#refresh();
    }
  }
}
