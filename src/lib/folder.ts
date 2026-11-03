// フォルダを選ぶ OS のダイアログ（Tauri のダイアログのプラグイン。画面設計書 7章「データ」・10.1）。
//
// 設定画面の保存先・書き出し先と、起動時の画面の「保存先を選び直す」で使う。選べなかったとき（取りやめた、
// ダイアログを開けなかった）は `null`。

import { open } from "@tauri-apps/plugin-dialog";

/** フォルダを選ぶ。`title` はダイアログの題、`defaultPath` は最初に開く場所。取りやめたら `null`。 */
export async function pickFolder(title: string, defaultPath?: string): Promise<string | null> {
  try {
    const picked = await open({ directory: true, multiple: false, title, defaultPath });
    return typeof picked === "string" ? picked : null;
  } catch {
    return null;
  }
}
