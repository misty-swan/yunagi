import { flushSync } from "svelte";
import { afterEach, expect, test } from "vitest";
import { MESSAGES, t } from "./i18n";
import { applyPrefs, defaultLang, minutesOf, prefs } from "./prefs.svelte";

afterEach(() => {
  prefs.apply({ ...DEFAULTS, locale: "ja" });
});

test("既定の言語は OS の言語の一覧で、日本語と英語のうち先に出てくる方", () => {
  expect(defaultLang(["ja-JP", "en-US"])).toBe("ja");
  expect(defaultLang(["fr-FR", "en-GB", "ja"])).toBe("en");
  expect(defaultLang(["de-DE"])).toBe("en");
  expect(defaultLang([])).toBe("en");
});

test("テーマは data-theme、言語は lang、フォントサイズは --fs-setting で文書に当てる", () => {
  const root = document.createElement("html");
  applyPrefs("dark", "en", 17, root);
  expect(root.dataset.theme).toBe("dark");
  expect(root.lang).toBe("en");
  expect(root.style.getPropertyValue("--fs-setting")).toBe("17");
  // OS に合わせるときは data-theme を付けない（tokens.css が OS の設定で選ぶ）。既定のサイズは変数を外す
  applyPrefs("system", "ja", 15, root);
  expect(root.hasAttribute("data-theme")).toBe(false);
  expect(root.lang).toBe("ja");
  expect(root.style.getPropertyValue("--fs-setting")).toBe("");
});

test("コアの設定を当てる。言語の設定が無ければ OS の言語のまま", () => {
  const before = prefs.lang;
  prefs.apply({
    dayBoundary: "05:00",
    meterStart: "08:30",
    meterEnd: "18:00",
    locale: null,
    updateCheck: false,
    theme: "dark",
    fontSize: 17,
    noteFont: "mono",
    layout: "B",
    carryoverReviewThreshold: 5,
    sidebarOpen: false,
  });
  expect(prefs.lang).toBe(before);
  expect(prefs.meterHours).toEqual({ start: 8 * 60 + 30, end: 18 * 60 });
  expect([prefs.theme, prefs.fontSize, prefs.noteFont, prefs.layout, prefs.reviewThreshold]).toEqual([
    "dark",
    17,
    "mono",
    "B",
    5,
  ]);
  expect([prefs.dayBoundary, prefs.updateCheck, prefs.sidebarOpen]).toEqual(["05:00", false, false]);
  prefs.apply({ ...DEFAULTS, locale: "en" });
  expect(prefs.lang).toBe("en");
  expect(minutesOf("23:59")).toBe(23 * 60 + 59);
  expect(minutesOf("24:00")).toBeNull();
  expect(minutesOf("9:00")).toBeNull();
});

const DEFAULTS = {
  dayBoundary: "04:00",
  meterStart: "09:00",
  meterEnd: "19:00",
  locale: null,
  updateCheck: true,
  theme: "system",
  fontSize: 15,
  noteFont: "body",
  layout: "A",
  carryoverReviewThreshold: 3,
  sidebarOpen: true,
} as const;

test("文言は言語で切り替わり、英字ラベルは日英で同じ", () => {
  prefs.lang = "ja";
  flushSync();
  expect(t().saveFailed).toBe("保存できていません");
  prefs.lang = "en";
  expect(t().saveFailed).toBe("Not saved");
  expect(MESSAGES.ja.labels).toBe(MESSAGES.en.labels);
});

test("日英の文言は同じキーをそろえて持つ", () => {
  const keys = (o: object): string[] =>
    Object.entries(o).flatMap(([k, v]) =>
      typeof v === "object" && v !== null ? keys(v).map((c) => `${k}.${c}`) : [k],
    );
  expect(keys(MESSAGES.en).sort()).toEqual(keys(MESSAGES.ja).sort());
});
