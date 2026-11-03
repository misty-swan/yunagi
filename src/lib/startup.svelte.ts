// 起動時の画面の操作（画面設計書 11章「起動時の画面」、アーキテクチャ設計書 4.3・6.2・6.3）。
// 見た目は src/startup/StartupScreen.svelte が持つ。
//
// - やり直す・新しく作る・保存先を選び直す・バックアップから戻すを、一度に1つだけ行う（`busy`）。
//   結果の状態は、コアの `startup` イベントで session に届く。
// - 失敗したら、どの操作の失敗かと中身を `error` に持つ（次の操作を始めると消す）。
// - バックアップから戻したら、退避した前のデータベースの場所を `restored` に持ち、利用者が
//   確かめる（`acknowledge`）まで起動時の画面に出し続ける。

import type { BackupEntry, StartupRestored } from "./bindings";
import { core, errorOf, type CallError } from "./core";

export type StartupAction = "retry" | "create" | "choose" | "restore";

export class StartupActions {
  /** 行っている操作。無ければ `null`。 */
  busy = $state<StartupAction | null>(null);
  /** 最後の操作の失敗。 */
  error = $state<{ action: StartupAction; error: CallError } | null>(null);
  /** 保存先の `backups/` にあるバックアップ（新しいものが先）。読む前と、読めなかったときは `null`。 */
  backups = $state<BackupEntry[] | null>(null);
  /** バックアップの一覧を読めなかった。 */
  backupsFailed = $state(false);
  /** バックアップから戻した結果。確かめるまで持つ。 */
  restored = $state<StartupRestored | null>(null);
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
    return this.#run("retry", () => core.retryOpen());
  }

  create() {
    return this.#run("create", () => core.createDatabase());
  }

  /** データベースのある既存のフォルダ `dir` を、保存先として選び直す。 */
  choose(dir: string) {
    return this.#run("choose", () => core.chooseDataDir(dir.trim()));
  }

  /** バックアップ `path` から戻す。 */
  restore(path: string) {
    return this.#run("restore", async () => {
      this.restored = await core.restoreAtStartup(path);
    });
  }

  /** 戻した結果を確かめた（整理モードへ進む）。 */
  acknowledge() {
    this.restored = null;
  }

  async #run(action: StartupAction, f: () => Promise<unknown>) {
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
  }
}
