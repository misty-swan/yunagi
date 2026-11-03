// バックアップの一覧の表示（種類と作った日時。画面設計書 7章・10.1）。起動時の画面と設定画面で共通。

import type { BackupEntry } from "./bindings";
import { shortDate } from "./dates";

/** 種類の文言（`StartupMessages` の `backupDaily` など）。 */
export interface BackupLabels {
  backupDaily: (date: string) => string;
  backupPreMigration: (from: number, to: number) => string;
  backupPreRestore: string;
}

/** 種類の表示（「定期 11.02 Mon」「更新前（スキーマ 2 → 3）」「復元前」）。 */
export function backupKindLabel(b: BackupEntry, m: BackupLabels): string {
  switch (b.kind.type) {
    case "daily":
      return m.backupDaily(shortDate(b.kind.date));
    case "pre_migration":
      return m.backupPreMigration(b.kind.from, b.kind.to);
    case "pre_restore":
      return m.backupPreRestore;
  }
}

function pad(n: number) {
  return String(n).padStart(2, "0");
}

/** 作った時刻（端末のタイムゾーンで `2026.11.03 14:05`）。 */
export function backupTime(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}.${pad(d.getMonth() + 1)}.${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}
