// 設定の画面（S-05）の開閉と区分（画面設計書 7章）。見た目は src/settings/SettingsPanel.svelte。
// 開いたのは、サイドバーの Settings・Mod+,・検索パレットの「設定を開く」・保存できていない原因の表示の
// 「保存先を変える」のどれか（どれも App の `openScreen` を通る）。閉じたら、開く直前にフォーカスの
// あった所へ戻る。

/** 設定の区分（画面設計書 7章。連携・ライセンス・このアプリについては、その機能を作るときに足す）。 */
export type SettingsSection = "general" | "display" | "operation" | "data" | "agents";

export const SETTINGS_SECTIONS: readonly SettingsSection[] = ["general", "display", "operation", "data", "agents"];

export class SettingsPanel {
  open = $state(false);
  section = $state<SettingsSection>("general");
  /** 開く直前にフォーカスのあった所（閉じたら戻す）。 */
  returnFocus: Element | null = null;

  /** 開く（`section` を指定すればその区分を、開いていれば区分だけ移す）。 */
  show(section?: SettingsSection) {
    if (!this.open) this.returnFocus = document.activeElement;
    this.open = true;
    if (section !== undefined) this.section = section;
  }

  close() {
    this.open = false;
  }
}
