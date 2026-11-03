// 起動時の画面の操作（画面設計書 10.1、アーキテクチャ設計書 4.3・6.2・6.3）。
// 見た目は src/startup/StartupScreen.svelte が持つ。
//
// - やり直す・新しく作る・保存先を選び直す・バックアップから戻すを、一度に1つだけ行う（`busy`）。
//   結果の状態は、コアの `startup` イベントで session に届く。
// - 失敗したら、どの操作の失敗かと中身を `error` に持つ（次の操作を始めると消す）。
// - バックアップから戻したら、コアの状態が `restored`（退避した前のデータベースの場所を持つ）になり、
//   利用者が確かめる（`acknowledge`）まで起動時の画面に出し続ける。状態はコアが持つので、画面を
//   読み込み直しても場所を示せる。
// - やり直す・選び直す・戻すの後は、バックアップの一覧を読み直す（同じ状態に戻っても、同期で
//   バックアップが届いていたり、使えなかったものが消えていたりする）。

import type { BackupEntry } from "./bindings";
import { core, errorOf, type CallError } from "./core";

export type StartupAction = "retry" | "create" | "choose" | "restore" | "acknowledge";

export class StartupActions {
  /** 行っている操作。無ければ `null`。 */
  busy = $state<StartupAction | null>(null);
  /** 最後の操作の失敗。 */
  error = $state<{ action: StartupAction; error: CallError } | null>(null);
  /** 保存先の `backups/` にあるバックアップ（新しいものが先）。読む前と、読めなかったときは `null`。 */
  backups = $state<BackupEntry[] | null>(null);
  /** バックアップの一覧を読めなかった。 */
  backupsFailed = $state(false);
  #loads = 0;

  /** バックアップの一覧を読み直す。 */
  async loadBackups() {
    const n = ++this.#loads;
    try {
      const list = await core.listBackups();
      if (n !== this.#loads) return;
      this.backups = list;
      this.backupsFailed = false;
    } catch {
      if (n !== this.#loads) return;
      this.backups = null;
      this.backupsFailed = true;
    }
  }

  retry() {
    return this.#run("retry", () => core.retryOpen(), true);
  }

  create() {
    return this.#run("create", () => core.createDatabase());
  }

  /** データベースのある既存のフォルダ `dir` を、保存先として選び直す。 */
  choose(dir: string) {
    return this.#run("choose", () => core.chooseDataDir(dir.trim()), true);
  }

  /** バックアップ `path` から戻す。 */
  restore(path: string) {
    return this.#run("restore", () => core.restoreAtStartup(path), true);
  }

  /** 戻した結果を確かめた（整理モードへ進む）。 */
  acknowledge() {
    return this.#run("acknowledge", () => core.acknowledgeRestore());
  }

  /** `action` を行う。`reload` なら、終わった後にバックアップの一覧を読み直す。 */
  async #run(action: StartupAction, f: () => Promise<unknown>, reload = false) {
    if (this.busy !== null) return;
    this.busy = action;
    this.error = null;
    try {
      await f();
    } catch (e) {
      this.error = { action, error: errorOf(e) };
    } finally {
      this.busy = null;
    }
    if (reload) void this.loadBackups();
  }
}
