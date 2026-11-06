// 画面が案内する、入れ直し方と問い合わせ先（画面設計書 10.1、ビルド・配布手順書 4章・5章、
// サポート・障害対応手順 1章）。URL とメールの宛先はここにだけ書く。
//
// 公式サイトのドメイン（getyunagi.com）と、配布・問い合わせ用の公開リポジトリ（misty-swan/yunagi）は
// 2026-11-06 に決まった（ビルド・配布手順書 1章・2章。YUN-11 で実際の値にした）。

import { isMacPlatform } from "./keys";

/** 公式サイト（install.sh・install.ps1 を置く）。 */
export const SITE = "https://getyunagi.com";
/** 不具合報告の窓口（公開リポジトリの GitHub Issues）。 */
export const ISSUES_URL = "https://github.com/misty-swan/yunagi/issues";
/** GitHub のアカウントが無い人の問い合わせ先。 */
export const SUPPORT_MAIL = "support@getyunagi.com";

export type Os = "mac" | "windows" | "linux";

/** 画面を動かしている OS（`navigator.userAgent` から決める。macOS かはキーの表示と同じ判定）。 */
export function currentOs(nav: Pick<Navigator, "userAgent"> = navigator): Os {
  if (isMacPlatform(nav)) return "mac";
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
