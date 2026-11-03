// 画面の骨組みから開く、ほかの画面（画面設計書 2章）。App.svelte が受ける。検索パレットと設定は App が
// 開き、変更履歴・ふりかえりは、それぞれの画面の課題で App の `openScreen` に開く処理を足す（それまでは
// 何もしない。パレットのコマンドも同じ入口を通る。画面設計書 6章）。

/**
 * - `palette`：検索パレット（S-04。上部のバーの検索欄、Mod+K）
 * - `settings`：設定（S-05。サイドバーの Settings、Mod+,）
 * - `agents`：エージェント変更履歴（S-06。サイドバーの Agents、上部のバーの「◆ 件数」）
 * - `archive`：ふりかえり（S-08。サイドバーの Archive、Mod+Shift+L）
 * - `dataSettings`：設定のデータ区分（上部のバーの、保存できていない原因の表示の「保存先を変える」）
 */
export type ShellCommand = "palette" | "settings" | "agents" | "archive" | "dataSettings";
