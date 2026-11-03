// 画面の骨組みから開く、ほかの画面（画面設計書 2章）。開く処理はそれぞれの画面の課題で作り、
// それまでは App.svelte が受けて何もしない。

/**
 * - `palette`：検索パレット（S-04。上部のバーの検索欄、Mod+K）
 * - `settings`：設定（S-05。サイドバーの Settings、Mod+,）
 * - `agents`：エージェント変更履歴（S-06。サイドバーの Agents、上部のバーの「◆ 件数」）
 * - `archive`：ふりかえり（S-08。サイドバーの Archive、Mod+Shift+L）
 */
export type ShellCommand = "palette" | "settings" | "agents" | "archive";
