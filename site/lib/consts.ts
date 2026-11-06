// 公式サイトが使う URL とメールの宛先。アプリが案内する値（src/lib/support.ts）と同じでなければならない
// （site/site.test.ts が確かめる）。決まりは docs/release/07-build-and-distribution.md 2章「公式サイトの置き場所」。

export const SITE = "https://getyunagi.com";
export const REPO = "https://github.com/misty-swan/yunagi";
export const ISSUES_URL = `${REPO}/issues`;
export const RELEASES_URL = `${REPO}/releases`;
/** 配布物の取得元（`<配布元>`。07 の 2章）。ファイルは `<配布元>/v<ver>/<ファイル名>` */
export const DOWNLOAD_BASE = `${REPO}/releases/download`;
export const SUPPORT_MAIL = "support@getyunagi.com";
export const SECURITY_MAIL = "security@getyunagi.com";

export const LANGS = ["ja", "en"] as const;
export type Lang = (typeof LANGS)[number];
