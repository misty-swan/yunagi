// 画面が案内する、入れ直し方と問い合わせ先（画面設計書 11章「起動時の画面」、ビルド・配布手順書 4章・5章、
// サポート・障害対応手順 1章）。URL とメールの宛先はここにだけ書く。
//
// 公式サイトのドメインと、配布・問い合わせ用の公開リポジトリはまだ決まっていない（YUN-11）。決まるまでは
// 設計書と同じ仮の表記（`<公式サイト>` など）を出し、決まったらここを書き換える。

/** 公式サイト（install.sh・install.ps1 を置く）。 */
export const SITE = "https://<公式サイト>";
/** 不具合報告の窓口（公開リポジトリの GitHub Issues）。 */
export const ISSUES_URL = "https://github.com/<公開リポジトリ>/issues";
/** GitHub のアカウントが無い人の問い合わせ先。 */
export const SUPPORT_MAIL = "support@<公式ドメイン>";

export type Os = "mac" | "windows" | "linux";

/** 画面を動かしている OS（`navigator.userAgent` から決める）。 */
export function currentOs(nav: Pick<Navigator, "userAgent"> = navigator): Os {
  if (/Mac|iPhone|iPad/.test(nav.userAgent)) return "mac";
  if (/Windows/.test(nav.userAgent)) return "windows";
  return "linux";
}

/**
 * インストールスクリプトで入れるコマンド（ビルド・配布手順書 4章）。`version` を渡すとその版を、
 * 渡さなければ最新の版を入れる（`YUNAGI_VERSION`）。
 */
export function installCommand(os: Os, version?: string): string {
  if (os === "windows") {
    const pin = version === undefined ? "" : `$env:YUNAGI_VERSION="${version}"; `;
    return `${pin}irm ${SITE}/install.ps1 | iex`;
  }
  const pin = version === undefined ? "" : `YUNAGI_VERSION=${version} `;
  return `curl -fsSL ${SITE}/install.sh | ${pin}sh`;
}
