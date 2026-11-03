// 持ち越しの画面（S-03。画面設計書 5章）の状態と操作。見た目は src/carryover/CarryoverPanel.svelte。
//
// - いつ出すか：コアの持ち越しの状態（`carryover_status`）で決める。データベースを開いたとき（起動も
//   前面に出したことに数える）と、アプリを前面に出したときに確かめ、`show` なら出す。後回しにした後の
//   `reshow_on_focus` は、再表示を記録できた（`mark_carryover_reshown` が `true`）ときだけ出す（1回だけ）。
//   `on_request` はパレットの「持ち越しを開く」（`openOnRequest`）でだけ開く。
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
  /** 前回の日が変わったので、一覧を読み直した。 */
  | "reloaded";

/** 行の一覧から、確定に渡すボタンの中身と見直しを作る。 */
export function decisionOf(
  rows: readonly CarryRow[],
  button: CarryButton,
): { selection: CarryoverSelection; reviews: CarryoverReviewChoice[] } {
  const reviews: CarryoverReviewChoice[] = [];
  for (const r of rows) {
    if (!r.review || r.choice === null) continue;
    if (r.choice === "drop") reviews.push({ id: r.item.id, review: { review: "drop" } });
    else reviews.push({ id: r.item.id, review: { review: "split", children: r.children.split(/\r?\n/) } });
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
  /** 前回の日（出している間）。 */
  from = $state<LogicalDate | null>(null);
  rows = $state<CarryRow[]>([]);
  /** 見直しの回数。持ち越し回数がこれ以上の印を `--warn` にする。 */
  reviewThreshold = $state(3);
  /** 今日の持ち越しが残っているか（パレットの「持ち越しを開く」を出すか）。確かめるまでは `null`。 */
  pending = $state<CarryoverStatus | null>(null);
  /** 確定・後回しを送っている途中。 */
  busy = $state(false);
  message = $state<CarryMessage | null>(null);

  /** 開くとき（表示を今日の整理モードにする）。 */
  #onOpen: () => void;
  #loads = 0;
  #checking: Promise<void> | null = null;
  #stopped = false;
  #unlisten: Promise<UnlistenFn | undefined>[] = [];

  constructor(onOpen: () => void) {
    this.#onOpen = onOpen;
    const listen = (p: Promise<UnlistenFn>) =>
      this.#unlisten.push(p.catch(() => undefined));
    listen(coreEvents.onChanged(() => void this.#refresh()));
    listen(coreEvents.onTodayChanged(() => void this.#refresh()));
  }

  /** データベースを開いたとき。起動は前面に出したことに数え、後回しの再表示もここで出す。 */
  started(): Promise<void> {
    return this.#check();
  }

  /** アプリを前面に出したとき。 */
  focused(): Promise<void> {
    return this.#check();
  }

  /** パレットの「持ち越しを開く」。持ち越しが残っていなければ何もしない。 */
  async openOnRequest(): Promise<void> {
    if (this.open) return;
    try {
      this.pending = await core.carryoverStatus();
    } catch {
      return;
    }
    if (this.pending !== null) await this.#show();
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

  /** ボタンで確定する。確定したら（終えていたときも）閉じ、結果を返す。 */
  async confirm(button: CarryButton): Promise<CarryoverConfirmed | null> {
    if (!this.open || this.busy) return null;
    if (this.from === null) {
      // 前回の日が無くなった（項目が消された）など、持ち越すものが無い
      this.#close();
      return null;
    }
    this.busy = true;
    this.message = null;
    try {
      const result = await core.confirmCarryover({ from: this.from, ...decisionOf(this.rows, button) });
      this.#close();
      return result;
    } catch (e) {
      const err = errorOf(e);
      if (err.kind === "conflict" && err.reason === "carryover_done") {
        // ほかの窓などで、もう終えていた
        this.#close();
      } else if (err.kind === "conflict" && err.reason === "previous_day_changed") {
        await this.#reload();
        this.message = "reloaded";
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
    for (const u of await Promise.all(this.#unlisten.splice(0))) u?.();
  }

  // 確かめている途中に、もう一度呼ばれたら（起動と前面に出したのが重なったときなど）、同じものを待つ
  #check(): Promise<void> {
    this.#checking ??= this.#doCheck().finally(() => (this.#checking = null));
    return this.#checking;
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
    this.message = null;
    this.from = null;
    this.rows = [];
    this.open = true;
    this.#onOpen();
    await this.#reload();
  }

  #close() {
    this.open = false;
    this.from = null;
    this.rows = [];
    this.message = null;
    this.pending = null;
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

  /** 一覧を読み直す（後から始めたものの答えだけを使う）。 */
  async #reload() {
    const n = ++this.#loads;
    let list: CarryoverList | null;
    try {
      list = await core.carryoverCandidates();
    } catch {
      return;
    }
    if (n !== this.#loads || !this.open) return;
    if (list === null) {
      this.rows = [];
      return;
    }
    const fresh = this.from !== list.from;
    if (fresh && this.from !== null) this.message = "reloaded";
    this.from = list.from;
    this.reviewThreshold = list.reviewThreshold;
    this.rows = mergeRows(this.rows, list, fresh);
  }
}
