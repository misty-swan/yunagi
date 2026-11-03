// 画面（WebView のページ）を読み込み直す。設定画面で、保存先の選び直しと復元の後に使う（別のデータになるので、
// 古い入力・版・Mod+Z の履歴が、新しいデータベースに書かれないようにする。画面設計書 7章）。
// テストで差し替えられるよう、ここにまとめる。

export function reloadPage(): void {
  window.location.reload();
}
