// 起動から今日のボードの表示までを、コアとつなぐ部品（見た目は持たない。画面の作業で使う）。
//
// - 開く処理の状態（startup）を追い、開けたら今日のボードを読む。
// - コアのデータが変わったら（changed。エージェントの変更も同じ）、ボードを読み込み直す
//   （docs/design/01-architecture.md 4章 6）。
// - 今日の論理日付が変わったら（today-changed）、新しい日のボードを読む。

import type { UnlistenFn } from "@tauri-apps/api/event";
import type { Board, LogicalDate, Startup } from "./bindings";
import { core, coreEvents, CoreError, type CallError } from "./core";

export interface Session {
  /** 開く処理の状態。問い合わせる前は `null`。 */
  readonly startup: Startup | null;
  /** 今日の論理日付。開くまでは `null`。 */
  readonly today: LogicalDate | null;
  /** 今日のボード。読むまでは `null`。 */
  readonly board: Board | null;
  /** 最後の読み込みの失敗。成功したら `null` に戻る。 */
  readonly error: CallError | null;
  /** 聞くのをやめる（やめ終わると解決する）。 */
  stop(): Promise<void>;
}

/** コアとつなぎ始める。 */
export function startSession(): Session {
  let startup = $state<Startup | null>(null);
  let today = $state<LogicalDate | null>(null);
  let board = $state<Board | null>(null);
  let error = $state<CallError | null>(null);
  let stopped = false;
  // 読み込みが行き違ったとき（続けて変わったとき）、最後に始めたものだけを使う
  let loads = 0;
  const unlisten: Promise<UnlistenFn>[] = [];

  const fail = (e: unknown) => {
    error = e instanceof CoreError ? e.error : { kind: "bad_request", message: String(e) };
  };

  async function load(date?: LogicalDate) {
    const n = ++loads;
    try {
      const day = date ?? (await core.today());
      const b = await core.getBoard(day);
      if (stopped || n !== loads) return;
      today = day;
      board = b;
      error = null;
    } catch (e) {
      if (!stopped && n === loads) fail(e);
    }
  }

  function onStartup(s: Startup) {
    startup = s;
    if (s.state === "ready") void load();
  }

  // 聞き始めてから状態を問い合わせる（その間に開き終わっても取りこぼさない）
  unlisten.push(coreEvents.onStartup(onStartup));
  unlisten.push(
    coreEvents.onChanged(() => {
      if (startup?.state === "ready") void load(today ?? undefined);
    }),
  );
  unlisten.push(coreEvents.onTodayChanged((p) => void load(p.today)));
  void Promise.all(unlisten)
    .then(() => core.startupStatus())
    .then((s) => {
      // イベントで先に新しい状態が届いていれば、それを使う
      if (!stopped && startup === null) onStartup(s);
    })
    .catch(fail);

  return {
    get startup() {
      return startup;
    },
    get today() {
      return today;
    },
    get board() {
      return board;
    },
    get error() {
      return error;
    },
    async stop() {
      stopped = true;
      const fns = await Promise.allSettled(unlisten);
      for (const r of fns) if (r.status === "fulfilled") r.value();
    },
  };
}
