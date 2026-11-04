// 画面の骨組みから開く、ほかの画面（画面設計書 2章）。App.svelte が受ける。検索パレット・設定・
// ふりかえりは App が開き、変更履歴は、その画面の課題で App の `openScreen` に開く処理を足す（それまでは
// 何もしない。パレットのコマンドも同じ入口を通る。画面設計書 6章）。

import type { LogicalDate } from "../lib/bindings";

/**
 * - `palette`：検索パレット（S-04。上部のバーの検索欄、Mod+K）
 * - `settings`：設定（S-05。サイドバーの Settings、Mod+,）
 * - `agents`：エージェント変更履歴（S-06。サイドバーの Agents、上部のバーの「◆ 件数」）
 * - `archive`：ふりかえり（S-08。サイドバーの Archive、Mod+Shift+L）
 * - `dataSettings`：設定のデータ区分（上部のバーの、保存できていない原因の表示の「保存先を変える」）
 */
export type ShellCommand = "palette" | "settings" | "agents" | "archive" | "dataSettings";

/**
 * ふりかえり（S-08）を開いている間の、外枠のキー・サイドバーの動き（画面設計書 9章）。開いていなければ
 * 外枠には渡さない（`null`）。
 */
export interface ArchiveControl {
  /** 選んでいる日（サイドバーの Recent で印を付ける）。決まるまでは `null`。 */
  date: LogicalDate | null;
  /** 日を選ぶ（サイドバーの Recent・Mod+T の今日）。 */
  onSelect: (date: LogicalDate) => void;
  /** 週を移る（Mod+[ は -1、Mod+] は 1）。 */
  onWeek: (weeks: number) => void;
  /** 元の画面へ戻る（Mod+Shift+L）。 */
  onClose: () => void;
  /** その日のボードを開く（サイドバーの Today）。 */
  onOpenDay: (date: LogicalDate) => void;
}
