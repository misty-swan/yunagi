// 設定画面の「データ」区分の操作（画面設計書 7章 S-05）：保存先の変更・既存のフォルダの選び直し、
// バックアップの一覧と復元、Markdown 書き出し。見た目は src/settings/DataSection.svelte が持つ。
//
// - 保存先の変更・選び直し・復元は、実行する前に確かめる画面（`flow`）を出す。選んだフォルダは、
//   コアでクラウド同期フォルダかを判定して、警告を添える（警告を出しても止めない）。
// - 選び直しと復元は、中身が別のデータになる。実行する前に、まだ保存していない入力を送り（復元前の
//   バックアップにも入る）、成功したら画面を読み込み直す（古い入力・版・Mod+Z の履歴が、新しい
//   データベースに書かれないように）。保存先の変更は中身が同じなので、読み込み直さない。
// - 一度に1つだけ行う（`busy`）。結果は `result` に持つ（次の操作を始めると消す）。

import type { BackupEntry, CloudService, LogicalDate } from "./bindings";
import { core, errorOf, type CallError } from "./core";
import { pickFolder } from "./folder";

/** 実行する前に確かめる操作。 */
export type DataFlow =
  | { kind: "move"; dir: string; cloud: CloudService | null }
  | { kind: "switch"; dir: string; cloud: CloudService | null }
  | { kind: "restore"; backup: BackupEntry };

/** 操作の結果。 */
export type DataResult =
  | { kind: "moved"; leftovers: string[] }
  | { kind: "exported"; count: number; dir: string }
  /** 書き出すものが無い期間だった（ファイルは作っていない）。 */
  | { kind: "nothing" }
  /** 保存できていない入力があるので、選び直し・復元をしなかった（`DataSettingsOptions.unsaved`）。 */
  | { kind: "unsaved" }
  | { kind: "failed"; action: "move" | "switch" | "restore" | "export"; error: CallError };

/** 書き出した結果（ファイルの数とフォルダ）を出しておく時間（ミリ秒）。 */
export const EXPORTED_SHOWN_MS = 8000;

export interface DataSettingsOptions {
  /** まだ保存していない入力を送り終える（保存に失敗していても、待ち終えて返る）。 */
  flush: () => Promise<unknown>;
  /**
   * 送り終えた後も、保存できていない入力が残っているか（保存に失敗している間）。残っていれば、選び直し・
   * 復元をしない（切り替えて読み込み直すと、その入力が失われ、復元前の退避にも入らないため）。
   */
  unsaved: () => boolean;
  /** 画面を読み込み直す。 */
  reload: () => void;
  /** 今日の論理日付（書き出しの期間の既定）。 */
  today: () => LogicalDate | null;
  /** フォルダを選ぶ（取りやめたら `null`）。 */
  pick?: (title: string, defaultPath?: string) => Promise<string | null>;
}

export class DataSettings {
  /** いまの保存先。読む前と、読めなかったときは `null`。 */
  dataDir = $state<string | null>(null);
  /** バックアップ（新しいものが先）。読む前と、読めなかったときは `null`。 */
  backups = $state<BackupEntry[] | null>(null);
  backupsFailed = $state(false);
  /** 実行する前に確かめている操作。 */
  flow = $state<DataFlow | null>(null);
  busy = $state<"move" | "switch" | "restore" | "export" | null>(null);
  result = $state<DataResult | null>(null);
  exportFrom = $state("");
  exportTo = $state("");
  /** 書き出し先に選んだフォルダ。 */
  exportDir = $state<string | null>(null);
  #opts: DataSettingsOptions;
  #loads = 0;
  #clear: ReturnType<typeof setTimeout> | undefined;

  constructor(opts: DataSettingsOptions) {
    this.#opts = opts;
  }

  /** 部品を片付けるとき、書き出しの結果を消す待ちをやめる。 */
  stop() {
    clearTimeout(this.#clear);
  }

  /** 保存先とバックアップの一覧を読む。書き出しの期間は、まだ決めていなければ今日にする。 */
  async load() {
    const n = ++this.#loads;
    const today = this.#opts.today();
    if (today !== null && this.exportFrom === "") this.exportFrom = today;
    if (today !== null && this.exportTo === "") this.exportTo = today;
    const [dir, list] = await Promise.allSettled([core.getDataDir(), core.listBackups()]);
    if (n !== this.#loads) return;
    this.dataDir = dir.status === "fulfilled" && typeof dir.value === "string" ? dir.value : null;
    if (list.status === "fulfilled" && Array.isArray(list.value)) {
      this.backups = list.value;
      this.backupsFailed = false;
    } else {
      this.backups = null;
      this.backupsFailed = true;
    }
  }

  get #pick() {
    return this.#opts.pick ?? pickFolder;
  }

  /** 保存先を変えるフォルダを選ばせて、確かめる画面を出す。 */
  async chooseMove(title: string) {
    await this.#choose("move", title);
  }

  /** データベースのある既存のフォルダを選ばせて、確かめる画面を出す。 */
  async chooseSwitch(title: string) {
    await this.#choose("switch", title);
  }

  async #choose(kind: "move" | "switch", title: string) {
    if (this.busy !== null) return;
    this.result = null;
    const dir = await this.#pick(title, this.dataDir ?? undefined);
    if (dir === null) return;
    let cloud: CloudService | null = null;
    try {
      cloud = (await core.checkDataDir(dir)) ?? null;
    } catch {
      // 判定できなければ警告しない（判定できない場所と同じ）
    }
    // ダイアログを開いている間にほかの操作が始まっていたら、その状態を上書きしない
    if (this.busy !== null || this.flow !== null) return;
    this.flow = { kind, dir, cloud };
  }

  /** バックアップからの復元を確かめる画面を出す。 */
  askRestore(backup: BackupEntry) {
    if (this.busy !== null) return;
    this.result = null;
    this.flow = { kind: "restore", backup };
  }

  /** 確かめる画面を閉じる（何もしない）。 */
  cancel() {
    if (this.busy === null) this.flow = null;
  }

  /** 確かめた操作を行う。 */
  async confirm() {
    const flow = this.flow;
    if (flow === null || this.busy !== null) return;
    this.busy = flow.kind;
    this.result = null;
    try {
      if (flow.kind === "move") {
        // 中身は同じなので、保存できていない入力があっても行う（保存先が外れて保存に失敗しているとき、
        // 保存先を変えて直すため）。入力は先に送り（送れなければ内容を持ったまま、移した後の保存先へ再試行する）
        await this.#opts.flush();
        const done = await core.changeDataDir(flow.dir);
        this.result = { kind: "moved", leftovers: done.leftovers };
        this.flow = null;
        await this.load();
      } else {
        // 別のデータになる。入力を送り終えてから行い、行ったら読み込み直す。送れなかった入力が残って
        // いれば（保存に失敗している）、何も変えずに断る
        await this.#opts.flush();
        if (this.#opts.unsaved()) {
          this.flow = null;
          this.result = { kind: "unsaved" };
          return;
        }
        if (flow.kind === "switch") await core.useDataDir(flow.dir);
        else await core.restoreBackup(flow.backup.path);
        this.flow = null;
        this.#opts.reload();
      }
    } catch (e) {
      this.flow = null;
      this.result = { kind: "failed", action: flow.kind, error: errorOf(e) };
    } finally {
      this.busy = null;
    }
  }

  /** 書き出し先のフォルダを選ばせる。 */
  async pickExportDir(title: string) {
    if (this.busy !== null) return;
    const dir = await this.#pick(title, this.exportDir ?? undefined);
    if (dir !== null) this.exportDir = dir;
  }

  /** 選んだ期間を、選んだフォルダに書き出す。 */
  async runExport() {
    if (this.busy !== null || this.exportDir === null || this.exportFrom === "" || this.exportTo === "") return;
    this.busy = "export";
    this.result = null;
    clearTimeout(this.#clear);
    try {
      const paths = await core.exportMarkdown(this.exportFrom, this.exportTo, this.exportDir);
      if (paths.length === 0) {
        this.result = { kind: "nothing" };
      } else {
        this.result = { kind: "exported", count: paths.length, dir: this.exportDir };
        // 書いたファイルの数とフォルダは、短時間だけ出す（画面設計書 7章）。次の書き出しは待ちを
        // やり直すので（上の clearTimeout）、古い待ちが新しい結果を消すことはない
        this.#clear = setTimeout(() => {
          if (this.result?.kind === "exported") this.result = null;
        }, EXPORTED_SHOWN_MS);
      }
    } catch (e) {
      this.result = { kind: "failed", action: "export", error: errorOf(e) };
    } finally {
      this.busy = null;
    }
  }
}
