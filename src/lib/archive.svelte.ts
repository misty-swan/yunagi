// ふりかえり（S-08。画面設計書 9章）の状態：開いているか、選んだ日、画面に出す内容。見た目は
// src/archive/Archive.svelte。
//
// - 開いたのは、サイドバーの Archive・Mod+Shift+L・検索パレットのコマンドのどれか（どれも App の
//   `openScreen` を通る）。閉じたら、開く直前にフォーカスのあった所へ戻る。
// - 選んだ日は、ふりかえりの中だけで持つ（表示している日 `session.date` は、Enter でその日を開くまで
//   動かさない。裏のボードを読み直して、自動保存の相手が入れ替わらないようにするため）。
// - 内容は、選んだ日が変わるたびにコアから読み直す（`get_archive`。週・Daily notes・プレビューを
//   同じ時点で読む）。読む前に、まだ保存していない入力を送る（今日の入力が古いまま出ないように）。
//   データが変わったとき（changed）と今日が変わったとき（today-changed）も読み直す。

import type { UnlistenFn } from "@tauri-apps/api/event";
import type { Archive, LogicalDate } from "./bindings";
import { core, coreEvents, errorOf, keepListening, LISTEN_RETRY_MS, type CallError } from "./core";
import { addDays } from "./dates";

export interface ArchiveDeps {
  /** 今日の論理日付（決まるまでは `null`）。 */
  today: () => LogicalDate | null;
  /** まだ保存していない入力を送り終えるまで待つ。 */
  flush: () => Promise<void>;
}

/** 読み込みに失敗したとき、読み直すまでの時間。 */
export const ARCHIVE_RETRY_MS = LISTEN_RETRY_MS;

export class ArchiveScreen {
  open = $state(false);
  /** 選んだ日。読み終えるまでは、選ぼうとしている日（最初に開いたときは、コアが決めるまで `null`）。 */
  sel = $state<LogicalDate | null>(null);
  /** 画面に出す内容。読むまでは `null`。 */
  data = $state<Archive | null>(null);
  /** 最後の読み込みの失敗。成功したら `null` に戻る。 */
  error = $state<CallError | null>(null);
  /** 増えたら、ふりかえりの枠へフォーカスを置く（サイドバーで日を選んだ後など）。 */
  focusSeq = $state(0);
  /** 開く直前にフォーカスのあった所（閉じたら戻す）。 */
  returnFocus: Element | null = null;

  readonly #deps: ArchiveDeps;
  #loads = 0;
  #retry: ReturnType<typeof setTimeout> | undefined;
  #stopped = false;
  readonly #unlisten: UnlistenFn[] = [];

  constructor(deps: ArchiveDeps) {
    this.#deps = deps;
  }

  /** データの変わり方（changed・today-changed）を聞き始める。 */
  start() {
    const listen = async (register: () => Promise<UnlistenFn>) => {
      const u = await keepListening(register, () => this.#stopped);
      if (u !== null) this.#unlisten.push(u);
    };
    void listen(() => coreEvents.onChanged(() => this.#refresh()));
    void listen(() => coreEvents.onTodayChanged(() => this.#refresh()));
  }

  stop() {
    this.#stopped = true;
    this.#loads++;
    clearTimeout(this.#retry);
    for (const u of this.#unlisten.splice(0)) u();
  }

  /**
   * 開く。`date` は最初に選ぶ日（今日より後は今日になる）。`null` なら、コアが決める（今日より前で
   * 記録のある最も新しい日。無ければ今日）。開いていれば、日だけを選び直す。
   */
  show(date: LogicalDate | null) {
    if (!this.open) {
      this.returnFocus = document.activeElement;
      this.open = true;
      this.data = null;
      this.error = null;
    }
    this.sel = date === null ? null : this.#clamp(date);
    this.focusSeq++;
    void this.#load();
  }

  /** 閉じる（待っている読み込みもやめる）。 */
  close() {
    this.open = false;
    this.sel = null;
    this.data = null;
    this.error = null;
    this.#loads++;
    clearTimeout(this.#retry);
  }

  /** 日を選ぶ。今日より後は選べない。 */
  select(date: LogicalDate) {
    if (!this.open) return;
    const today = this.#today();
    if (today !== null && date > today) return;
    // すでに選んでいて読み終えている日は、読み直さない
    if (date === this.sel && this.data?.date === date) return;
    this.sel = date;
    void this.#load();
  }

  /** 選んだ日を `days` 日ずらす（今日で止まる）。 */
  moveDay(days: number) {
    this.#move(days);
  }

  /** 週を `weeks` 週ずらす（同じ曜日へ。今日より先へは行かず、今日で止まる）。 */
  moveWeek(weeks: number) {
    this.#move(7 * weeks);
  }

  #today(): LogicalDate | null {
    return this.#deps.today() ?? this.data?.today ?? null;
  }

  #clamp(date: LogicalDate): LogicalDate {
    const today = this.#today();
    return today !== null && date > today ? today : date;
  }

  #move(days: number) {
    if (!this.open) return;
    const base = this.sel ?? this.data?.date ?? this.#today();
    if (base === null) return;
    const next = this.#clamp(addDays(base, days));
    if (next === this.sel && this.data?.date === next) return;
    this.sel = next;
    void this.#load();
  }

  #refresh() {
    if (this.open && !this.#stopped) void this.#load();
  }

  async #load() {
    const n = ++this.#loads;
    clearTimeout(this.#retry);
    try {
      await this.#deps.flush();
    } catch {
      // 保存できなかった入力は、保存の失敗の表示が伝える。読み込みは続ける
    }
    if (n !== this.#loads) return;
    try {
      const a = await core.getArchive(this.sel);
      if (n !== this.#loads || !this.open) return;
      this.data = a;
      this.sel = a.date;
      this.error = null;
    } catch (e) {
      if (n !== this.#loads || !this.open) return;
      this.error = errorOf(e);
      // 読めるまで、間を空けて読み直す
      this.#retry = setTimeout(() => {
        if (n === this.#loads) this.#refresh();
      }, ARCHIVE_RETRY_MS);
    }
  }
}
