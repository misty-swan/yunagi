// 持ち越しの画面（S-03。画面設計書 5章）の状態と操作。見た目は src/carryover/CarryoverPanel.svelte。
//
// - いつ出すか：コアの持ち越しの状態（`carryover_status`）で決める。データベースを開いたとき（起動も
//   前面に出したことに数える）と、アプリを前面に出したときに確かめ、`show` なら出す。後回しにした後の
//   `reshow_on_focus` は、再表示を記録できた（`mark_carryover_reshown` が `true`）ときだけ出す（1回だけ）。
//   `on_request` はパレットの「持ち越しを開く」（`openOnRequest`）でだけ開く。
// - 開くときは、まだ保存していない入力を送り終えてから一覧を読む（送る前の本文・状態を写さないため）。
//   読み終えるまでは確定できない。読めなければ知らせて、間を空けて読み直す。
// - 出している間にデータが変わったら（changed。エージェントの変更を含む）、一覧を読み直して選択を
//   引き継ぐ：消えた項目は選択・見直し・子の入力ごと捨て、新しく加わった項目はチェックを付けずに加える。
//   前回の日が変わったら、開き直したものとして読み直す（全項目にチェック）。
// - 確定を Mod+Z で取り消すかはまだ決めていない（画面設計書 11章）。`confirm` は確定の結果
//   （作った項目・取りやめた項目）を返すので、取り消すことにしたら、それを取り消しの履歴に持つ。

import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  CarryoverConfirmed,
  CarryoverList,
  CarryoverReviewChoice,
  CarryoverSelection,
  CarryoverStatus,
  Item,
  LogicalDate,
} from "./bindings";
import { core, coreEvents, errorOf } from "./core";

/** 一覧を読めなかったとき・イベントを聞き始められなかったとき、やり直すまでの時間。 */
export const CARRY_RETRY_MS = 1000;

/** 見直しの選択（分割する／やめる）。 */
export type ReviewChoice = "split" | "drop";

/** 持ち越しの画面の1行。 */
export interface CarryRow {
  /** 前回の日の項目（読み直すたびに最新にする）。 */
  item: Item;
  /** 見直しの選択肢を添えるか（持ち越し回数が見直しの回数以上の TODO）。 */
  review: boolean;
  /** チェック（「選んだものを持ち越す」で持ち越す）。 */
  checked: boolean;
  /** 見直しの選択。選ぶとチェックを外し、チェックを付けると外す。 */
  choice: ReviewChoice | null;
  /** 「分割する」の子の TODO（1行に1件）。 */
  children: string;
}

/** 押したボタン。 */
export type CarryButton = "selected" | "all" | "none";

/** 画面に出す知らせ。 */
export type CarryMessage =
  /** 確定に失敗した（保存先の問題など）。もう一度押せる。 */
  | "failed"
  /** 見直しの選択か分割の子を、コアが受け付けなかった。 */
  | "invalid"
  /** 一覧を読めなかった（読み直している）。 */
  | "loadFailed"
  /** 前回の日が変わったので、一覧を読み直した。 */
  | "reloaded";

/** 分割の子の入力欄の本文を、1行ずつに分ける（CR・CRLF も行の区切りにする。コアは改行のある本文を断る）。 */
export function splitChildren(text: string): string[] {
  return text.split(/\r\n|\r|\n/);
}

/** 行の一覧から、確定に渡すボタンの中身と見直しを作る。 */
export function decisionOf(
  rows: readonly CarryRow[],
  button: CarryButton,
): { selection: CarryoverSelection; reviews: CarryoverReviewChoice[] } {
  const reviews: CarryoverReviewChoice[] = [];
  for (const r of rows) {
    if (!r.review || r.choice === null) continue;
    if (r.choice === "drop") reviews.push({ id: r.item.id, review: { review: "drop" } });
    else reviews.push({ id: r.item.id, review: { review: "split", children: splitChildren(r.children) } });
  }
  let selection: CarryoverSelection;
  if (button === "all") selection = { select: "all" };
  else if (button === "none") selection = { select: "none" };
  else {
    // 子を1件も書かなかった「分割する」は、見直しを選ばなかったものとして持ち越す（チェックは
    // 外れているが、分けるつもりの項目を前の日に置き去りにしないため）。子のある分割はコアが先に扱う
    const ids = rows.filter((r) => r.checked || (r.review && r.choice === "split")).map((r) => r.item.id);
    selection = { select: "selected", ids };
  }
  return { selection, reviews };
}

/**
 * 読み直した一覧 `list` に、今の行 `rows` の選択を引き継ぐ。`fresh` なら（開いたとき・前回の日が
 * 変わったとき）全項目にチェックを付ける。
 */
export function mergeRows(rows: readonly CarryRow[], list: CarryoverList, fresh: boolean): CarryRow[] {
  const known = new Map(rows.map((r) => [r.item.id, r]));
  return list.items.map(({ item, review }) => {
    const prev = fresh ? undefined : known.get(item.id);
    if (prev === undefined) return { item, review, checked: fresh, choice: null, children: "" };
    // 見直しの選択肢が出なくなった（回数の設定が変わった）ら、選んでいた見直しを捨てる
    if (!review) return { item, review, checked: prev.checked || prev.choice !== null, choice: null, children: "" };
    return { ...prev, item, review };
  });
}

export class Carryover {
  /** 画面を出しているか。 */
  open = $state(false);
  /** 一覧を読み終えたか（読み終えるまでは確定できない）。 */
  loaded = $state(false);
  /** 前回の日（読み終えた後。前回の日が無くなったら `null`）。 */
  from = $state<LogicalDate | null>(null);
  rows = $state<CarryRow[]>([]);
  /** 見直しの回数。持ち越し回数がこれ以上の印を `--warn` にする。 */
  reviewThreshold = $state(3);
  /** 今日の持ち越しが残っているか（パレットの「持ち越しを開く」を出すか）。確かめるまでは `null`。 */
  pending = $state<CarryoverStatus | null>(null);
  /** 確定・後回しを送っている途中。 */
  busy = $state(false);
  message = $state<CarryMessage | null>(null);
  /** 開く直前にフォーカスのあった所（閉じたら画面が戻す）。 */
  returnFocus: Element | null = null;

  /** 開くとき（表示を今日の整理モードにし、まだ保存していない入力を送り終えたら解決する）。 */
  #onOpen: () => Promise<void> | void;
  #loads = 0;
  #retry: ReturnType<typeof setTimeout> | undefined;
  #checking: Promise<void> | null = null;
  #stopped = false;
  #unlisten: UnlistenFn[] = [];
  #listening: Promise<unknown>;

  constructor(onOpen: () => Promise<void> | void) {
    this.#onOpen = onOpen;
    this.#listening = Promise.all([
      this.#keepListening(() => coreEvents.onChanged(() => void this.#refresh())),
      this.#keepListening(() => coreEvents.onTodayChanged(() => void this.#refresh())),
    ]);
  }

  /** データベースを開いたとき。起動は前面に出したことに数え、後回しの再表示もここで出す。 */
  started(): Promise<void> {
    return this.#check(() => this.#doCheck());
  }

  /** アプリを前面に出したとき。 */
  focused(): Promise<void> {
    return this.#check(() => this.#doCheck());
  }

  /** パレットの「持ち越しを開く」。持ち越しが残っていなければ何もしない。 */
  openOnRequest(): Promise<void> {
    return this.#check(async () => {
      if (this.open || this.#stopped) return;
      try {
        this.pending = await core.carryoverStatus();
      } catch {
        return;
      }
      if (this.pending !== null) await this.#show();
    });
  }

  /** 行のチェックを切り替える。付けると見直しの選択を外す。 */
  toggle(row: CarryRow, checked = !row.checked) {
    row.checked = checked;
    if (checked) row.choice = null;
  }

  /** 見直しを選ぶ（同じものをもう一度選ぶと外し、チェックを戻す）。 */
  choose(row: CarryRow, choice: ReviewChoice) {
    if (!row.review) return;
    if (row.choice === choice) {
      row.choice = null;
      row.checked = true;
    } else {
      row.choice = choice;
      row.checked = false;
    }
  }

  /** ボタンで確定する。確定したら（終えていたときも）閉じ、結果を返す。読み終える前は何もしない。 */
  async confirm(button: CarryButton): Promise<CarryoverConfirmed | null> {
    if (!this.open || this.busy || !this.loaded) return null;
    if (this.from === null) {
      // 前回の日が無くなった（項目が消された）ので、持ち越すものが無い
      this.#close();
      void this.#refreshPending();
      return null;
    }
    this.busy = true;
    this.message = null;
    try {
      const result = await core.confirmCarryover({ from: this.from, ...decisionOf(this.rows, button) });
      this.#close();
      this.pending = null;
      return result;
    } catch (e) {
      const err = errorOf(e);
      if (err.kind === "conflict" && err.reason === "carryover_done") {
        // ほかの窓などで、もう終えていた
        this.#close();
        this.pending = null;
      } else if (err.kind === "conflict" && err.reason === "previous_day_changed") {
        await this.#reload();
        if (this.open) this.message = "reloaded";
      } else if (err.kind === "invalid_argument") {
        this.message = "invalid";
      } else {
        this.message = "failed";
      }
      return null;
    } finally {
      this.busy = false;
    }
  }

  /** 後回し（Esc）。記録に失敗しても閉じる（次に前面に出したとき、もう一度出る）。 */
  async postpone(): Promise<void> {
    if (!this.open || this.busy) return;
    this.busy = true;
    try {
      await core.postponeCarryover();
    } catch {
      // 閉じるのを優先する
    } finally {
      this.busy = false;
    }
    this.#close();
    void this.#refreshPending();
  }

  /** 聞くのをやめる。 */
  async stop(): Promise<void> {
    this.#stopped = true;
    clearTimeout(this.#retry);
    for (const u of this.#unlisten.splice(0)) u();
    // 聞き始めている途中のものは、終わったところで（stopped を見て）やめる
    await this.#listening;
  }

  // 聞き始められなければ、聞き始められるまでやり直す（session.svelte.ts と同じ）
  async #keepListening(register: () => Promise<UnlistenFn>) {
    while (!this.#stopped) {
      try {
        const u = await register();
        if (this.#stopped) u();
        else this.#unlisten.push(u);
        return;
      } catch {
        await new Promise((resolve) => setTimeout(resolve, CARRY_RETRY_MS));
      }
    }
  }

  // 確かめて開く処理は1つずつ行う（起動と前面に出したのが重なったとき、パレットから開くのと
  // 重なったときに、1回だけの再表示を記録したのに出さない、ということが無いように）
  #check(run: () => Promise<void>): Promise<void> {
    const prev = this.#checking ?? Promise.resolve();
    const next = prev.then(run, run);
    const tracked = next.finally(() => {
      if (this.#checking === tracked) this.#checking = null;
    });
    this.#checking = tracked;
    return tracked;
  }

  async #doCheck() {
    if (this.open || this.#stopped) return;
    let status: CarryoverStatus | null;
    try {
      status = await core.carryoverStatus();
    } catch {
      return;
    }
    this.pending = status;
    if (status === null || this.open || this.#stopped) return;
    if (status.prompt === "show") await this.#show();
    else if (status.prompt === "reshow_on_focus") {
      let reshown = false;
      try {
        reshown = await core.markCarryoverReshown();
      } catch {
        return;
      }
      if (reshown) await this.#show();
    }
  }

  async #show() {
    if (this.open || this.#stopped) return;
    this.returnFocus = document.activeElement;
    this.message = null;
    this.loaded = false;
    this.from = null;
    this.rows = [];
    this.open = true;
    try {
      await this.#onOpen();
    } catch {
      // 送れなかった入力は、ふだんの自動保存が再試行する
    }
    await this.#reload();
  }

  #close() {
    clearTimeout(this.#retry);
    this.#loads++;
    this.open = false;
    this.loaded = false;
    this.from = null;
    this.rows = [];
    this.message = null;
  }

  /** データが変わった・今日が変わったとき。 */
  async #refresh() {
    if (this.#stopped) return;
    if (this.open) await this.#reload();
    else await this.#refreshPending();
  }

  async #refreshPending() {
    try {
      const status = await core.carryoverStatus();
      if (!this.#stopped) this.pending = status;
    } catch {
      // 次に確かめたときに読む
    }
  }

  /** 一覧を読み直す（後から始めたものの答えだけを使う）。読めなければ間を空けて読み直す。 */
  async #reload() {
    clearTimeout(this.#retry);
    const n = ++this.#loads;
    let list: CarryoverList | null;
    try {
      list = await core.carryoverCandidates();
    } catch {
      if (n !== this.#loads || !this.open || this.#stopped) return;
      this.message = "loadFailed";
      this.#retry = setTimeout(() => {
        if (n === this.#loads && this.open && !this.#stopped) void this.#reload();
      }, CARRY_RETRY_MS);
      return;
    }
    if (n !== this.#loads || !this.open) return;
    if (this.message === "loadFailed") this.message = null;
    const wasLoaded = this.loaded;
    this.loaded = true;
    if (list === null) {
      // 前回の日が無くなった（前の日の項目・ノートが消された）
      this.from = null;
      this.rows = [];
      return;
    }
    const fresh = !wasLoaded || this.from !== list.from;
    if (fresh && wasLoaded) this.message = "reloaded";
    this.from = list.from;
    this.reviewThreshold = list.reviewThreshold;
    this.rows = mergeRows(this.rows, list, fresh);
  }
}
