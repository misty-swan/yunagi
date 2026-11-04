// 設定の、画面が使う値：テーマ・言語・フォントサイズ・ノートの書体・レイアウト・凪メーターの時間帯・
// 持ち越し見直しの回数・サイドバーの開閉・更新の自動確認・ショートカットの割当・Vim モード（要件 F-12、
// 画面設計書 7章、デザインガイド
// 3章・4章・7章）。
//
// 値の正本はコアの setting（データモデル設計書 3章）。起動のとき、外枠を出す前に読み込む
// （`SettingsStore.load`。src/lib/settings.svelte.ts）。それまでは既定の値（テーマは OS に合わせる、
// 言語は OS の言語）を持つ。変えるのは設定画面で、`SettingsStore.save` がこの値を変えてから保存する。

import type { Layout, Locale, NoteFont, Settings, Theme } from "./bindings";

export type { Layout, NoteFont, Theme };

/** 画面の言語。 */
export type Lang = Locale;

/** フォントサイズ（px）の範囲と既定値（コアの `domain::settings` と同じ。デザインガイド 4章）。 */
export const FONT_SIZE_MIN = 13;
export const FONT_SIZE_MAX = 19;
export const FONT_SIZE_DEFAULT = 15;

/** 持ち越し見直しの回数の既定値（コアの `DEFAULT_REVIEW_THRESHOLD`）。 */
export const REVIEW_THRESHOLD_DEFAULT = 3;

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

/** `HH:MM` を、その日の始まりからの分にする。読めなければ `null`。 */
export function minutesOf(hhmm: string): number | null {
  const m = /^([01]\d|2[0-3]):([0-5]\d)$/.exec(hhmm);
  return m === null ? null : Number(m[1]) * 60 + Number(m[2]);
}

class Prefs {
  theme = $state<Theme>("system");
  lang = $state<Lang>(defaultLang());
  layout = $state<Layout>("A");
  noteFont = $state<NoteFont>("body");
  fontSize = $state(FONT_SIZE_DEFAULT);
  /** 持ち越し回数をこの値以上で `--warn` にする（見直しの選択肢が出る回数と同じ）。 */
  reviewThreshold = $state(REVIEW_THRESHOLD_DEFAULT);
  /** 日付の切り替わり時刻（`HH:MM`）。 */
  dayBoundary = $state("04:00");
  /** 凪メーターの始業・終業（`HH:MM`）。 */
  meterStart = $state("09:00");
  meterEnd = $state("19:00");
  updateCheck = $state(true);
  /** 広いウィンドウの整理モードでのサイドバーの開閉。 */
  sidebarOpen = $state(true);
  /**
   * ショートカットの割当：操作の名前から割当の文字列へ。初期割当から変えたものだけ（keymap.ts）。
   * 変えるときは新しい値を代入する（中身は書き換えない。失敗したときに前の値へ戻せるように）。
   */
  shortcuts = $state.raw<Record<string, string>>({});
  /** Vim モード。 */
  vimMode = $state(false);
  /** Vim モードの挿入モードに入ったとき、直前の入力状態に戻す。 */
  vimImeRestore = $state(true);

  /** 凪メーターの時間帯（分）。 */
  get meterHours(): { start: number; end: number } {
    return { start: minutesOf(this.meterStart) ?? 9 * 60, end: minutesOf(this.meterEnd) ?? 19 * 60 };
  }

  /** コアから読んだ設定を当てる。言語は、設定が無ければ OS の言語のまま。 */
  apply(s: Settings) {
    this.theme = s.theme;
    if (s.locale !== null) this.lang = s.locale;
    this.layout = s.layout;
    this.noteFont = s.noteFont;
    this.fontSize = s.fontSize;
    this.reviewThreshold = s.carryoverReviewThreshold;
    this.dayBoundary = s.dayBoundary;
    this.meterStart = s.meterStart;
    this.meterEnd = s.meterEnd;
    this.updateCheck = s.updateCheck;
    this.sidebarOpen = s.sidebarOpen;
    this.shortcuts = { ...s.shortcuts };
    this.vimMode = s.vimMode;
    this.vimImeRestore = s.vimImeRestore;
  }
}

/** いまの設定（画面全体で1つ）。 */
export const prefs = new Prefs();

/**
 * 設定を文書に当てる：テーマは `data-theme`（OS に合わせるときは付けない。tokens.css）、言語は `lang`、
 * フォントサイズは `--fs-setting`（本文の px。全体が `--u` に比例して拡縮する。src/styles/base.css、
 * デザインガイド 4章）。テーマの切り替えは CSS の変数を変えるだけで、アニメーションは付けない。
 */
export function applyPrefs(
  theme: Theme,
  lang: Lang,
  fontSize: number = FONT_SIZE_DEFAULT,
  root: HTMLElement = document.documentElement,
) {
  if (theme === "system") root.removeAttribute("data-theme");
  else root.dataset.theme = theme;
  root.lang = lang;
  if (fontSize === FONT_SIZE_DEFAULT) root.style.removeProperty("--fs-setting");
  else root.style.setProperty("--fs-setting", String(fontSize));
}
