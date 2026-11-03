// 起動から、表示している日のボードまでを、コアとつなぐ部品（見た目は持たない。画面の作業で使う）。
//
// - 開く処理の状態（startup）を追い、開けたら今日のボードを読む。`show` でほかの日に移る。
// - ボードと一緒に、画面の骨組みの件数（overview：凪メーター・Recent・未確認の変更）を読む。
// - コアのデータが変わったら（changed。エージェントの変更も同じ）、読み込み直す
//   （docs/design/01-architecture.md 4章 6）。
// - 今日の論理日付が変わったら（today-changed）、今日を表示していたなら新しい今日へ移り、
//   ほかの日を表示していたならその日のまま読み込み直す（Recent と凪メーターの時刻が変わるため。
//   画面設計書 3章「上部のバー」）。

import type { UnlistenFn } from "@tauri-apps/api/event";
import type { Board, LogicalDate, Overview, Startup } from "./bindings";
import { core, coreEvents, errorOf, type CallError } from "./core";

export interface Session {
  /** 開く処理の状態。問い合わせる前は `null`。 */
  readonly startup: Startup | null;
  /** 今日の論理日付。開くまでは `null`。 */
  readonly today: LogicalDate | null;
  /** 表示している日。読むまでは `null`。 */
  readonly date: LogicalDate | null;
  /**
   * 表示しようとしている日（`show` で移って読んでいる途中の日。読み終えると `date` と同じ）。
   * 前日・翌日への移動を続けて押したとき、ここから数える。決まるまでは `null`。
   */
  readonly showing: LogicalDate | null;
  /** 表示している日のボード。読むまでは `null`。 */
  readonly board: Board | null;
  /** 表示している日の、画面の骨組みの件数。読むまでは `null`。 */
  readonly overview: Overview | null;
  /** 最後の読み込みの失敗。成功したら `null` に戻る。 */
  readonly error: CallError | null;
  /** 日 `date` を表示する（読み終わると `date`・`board`・`overview` が変わる）。 */
  show(date: LogicalDate): void;
  /** 表示している日を読み込み直す（保存を捨てた後など）。 */
  reload(): void;
  /** 聞くのをやめる（やめ終わると解決する）。 */
  stop(): Promise<void>;
}

/** 開く処理の状態の問い合わせや、イベントを聞き始めるのに失敗したとき、やり直すまでの時間。 */
export const RETRY_MS = 1000;

/** コアとつなぎ始める。 */
export function startSession(): Session {
  let startup = $state<Startup | null>(null);
  let today = $state<LogicalDate | null>(null);
  let date = $state<LogicalDate | null>(null);
  let board = $state<Board | null>(null);
  let overview = $state<Overview | null>(null);
  let error = $state<CallError | null>(null);
  let stopped = false;
  // 読み込みが行き違ったとき（続けて変わったとき）、最後に始めたものだけを使う
  let loads = 0;
  // 表示する日（読んでいる・読み終えた日）。日を移って読んでいる途中に changed が届いても、
  // 前の日を読み直さないよう、読み始めた時点で進める。まだ決まっていなければ今日を読む
  let target = $state<LogicalDate | undefined>(undefined);
  let overviewRetry: ReturnType<typeof setTimeout> | undefined;
  // 知っている最新の今日（`today` より先に、core.today() の答えと today-changed で進める）。
  // 最初の読み込みを終える前に日付が変わっても、今日を表示していたかを見分けるため
  let knownToday: LogicalDate | undefined;
  const unlisten: UnlistenFn[] = [];

  const fail = (e: unknown) => {
    error = errorOf(e);
  };

  async function load(next?: LogicalDate) {
    const n = ++loads;
    clearTimeout(overviewRetry);
    if (next !== undefined) target = next;
    try {
      let day = target;
      if (day === undefined) {
        day = await core.today();
        knownToday ??= day;
        if (n === loads) target = day;
      }
      // ボードは、件数（overview）が読めなくても出す
      const [b, o] = await Promise.allSettled([core.getBoard(day), core.getOverview(day)]);
      if (stopped || n !== loads) return;
      if (b.status === "rejected") throw b.reason;
      date = day;
      board = b.value;
      if (knownToday !== undefined) today = knownToday;
      if (o.status === "fulfilled") {
        overview = o.value;
        // Recent の先頭はいつも今日
        knownToday = o.value.recent[0]?.date ?? knownToday;
        if (knownToday !== undefined) today = knownToday;
        error = null;
      } else {
        // 前の日の件数を、表示している日のものとして出さない。読めるまで間を空けて読み直す
        overview = null;
        fail(o.reason);
        overviewRetry = setTimeout(() => {
          if (!stopped && n === loads) void load();
        }, RETRY_MS);
      }
    } catch (e) {
      if (!stopped && n === loads) fail(e);
    }
  }

  function onTodayChanged(next: LogicalDate) {
    // 今日（またはまだ決まっていない日）を表示していたなら、新しい今日へ移る
    const follow = target === undefined || target === knownToday;
    knownToday = next;
    today = next;
    if (startup?.state === "ready") void load(follow ? next : target);
    else if (follow) target = next;
  }

  function onStartup(s: Startup) {
    startup = s;
    if (s.state === "ready") void load(target);
  }

  // 聞き始められなければ、聞き始められるまでやり直す
  async function keepListening(register: () => Promise<UnlistenFn>) {
    while (!stopped) {
      try {
        const u = await register();
        if (stopped) u();
        else unlisten.push(u);
        return;
      } catch (e) {
        fail(e);
        await new Promise((resolve) => setTimeout(resolve, RETRY_MS));
      }
    }
  }

  // 問い合わせに失敗したら、届くまで問い合わせ直す（聞き始める前に開き終わっていると、
  // startup はもう届かないため）
  async function askStartup() {
    try {
      const s = await core.startupStatus();
      // イベントで先に新しい状態が届いていれば、それを使う
      if (!stopped && startup === null) onStartup(s);
    } catch (e) {
      if (stopped || startup !== null) return;
      fail(e);
      setTimeout(() => void askStartup(), RETRY_MS);
    }
  }

  // 聞き始めてから状態を問い合わせる（その間に開き終わっても取りこぼさない）
  const listening = Promise.all([
    keepListening(() => coreEvents.onStartup(onStartup)),
    keepListening(() =>
      coreEvents.onChanged(() => {
        if (startup?.state === "ready") void load(target);
      }),
    ),
    keepListening(() => coreEvents.onTodayChanged((p) => onTodayChanged(p.today))),
  ]);
  void listening.then(askStartup);

  return {
    get startup() {
      return startup;
    },
    get today() {
      return today;
    },
    get date() {
      return date;
    },
    get showing() {
      return target ?? null;
    },
    get board() {
      return board;
    },
    get overview() {
      return overview;
    },
    get error() {
      return error;
    },
    show(next: LogicalDate) {
      if (startup?.state === "ready") void load(next);
      else target = next;
    },
    reload() {
      if (startup?.state === "ready") void load(target);
    },
    async stop() {
      stopped = true;
      clearTimeout(overviewRetry);
      for (const u of unlisten.splice(0)) u();
      // 聞き始めている途中のものは、終わったところで（stopped を見て）やめる
      await listening;
    },
  };
}
