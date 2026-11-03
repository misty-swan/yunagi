// 表示の設定：テーマと言語とレイアウト（要件 F-12、画面設計書 7章、デザインガイド 3章・7章）。
//
// 設定画面（S-05）と、設定の保存（コアの setting）はまだ無い。それまでは、起動のたびに既定の値
// （テーマは OS に合わせる、言語は OS の言語）で始める。設定画面はこの値を変え、保存した値で始める。

/** テーマ：OS に合わせる／ライト／ダーク（既定は OS に合わせる）。 */
export type Theme = "system" | "light" | "dark";

/** 画面の言語。 */
export type Lang = "ja" | "en";

/** 整理モードのレイアウト（画面設計書 3章「レイアウト」）：A. 2段（既定）、B. 3列、C. 縦積み。 */
export type Layout = "A" | "B" | "C";

/**
 * OS（WebView）の言語から決める既定の言語：優先する言語の一覧で、日本語と英語のうち先に出てくる方。
 * どちらも無ければ英語。
 */
export function defaultLang(languages: readonly string[] = navigator.languages ?? []): Lang {
  for (const l of languages) {
    const base = l.toLowerCase().split("-")[0];
    if (base === "ja" || base === "en") return base;
  }
  return "en";
}

class Prefs {
  theme = $state<Theme>("system");
  lang = $state<Lang>(defaultLang());
  layout = $state<Layout>("A");
}

/** いまの表示の設定（画面全体で1つ）。 */
export const prefs = new Prefs();

/**
 * 設定を文書に当てる：テーマは `data-theme`（OS に合わせるときは付けない。tokens.css）、言語は `lang`。
 * テーマの切り替えは CSS の変数を変えるだけで、アニメーションは付けない。
 */
export function applyPrefs(theme: Theme, lang: Lang, root: HTMLElement = document.documentElement) {
  if (theme === "system") root.removeAttribute("data-theme");
  else root.dataset.theme = theme;
  root.lang = lang;
}
