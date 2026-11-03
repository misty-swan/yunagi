import { flushSync } from "svelte";
import { afterEach, expect, test } from "vitest";
import { MESSAGES, t } from "./i18n";
import { applyPrefs, defaultLang, prefs } from "./prefs.svelte";

afterEach(() => {
  prefs.lang = "ja";
  prefs.theme = "system";
});

test("既定の言語は OS の言語の一覧で、日本語と英語のうち先に出てくる方", () => {
  expect(defaultLang(["ja-JP", "en-US"])).toBe("ja");
  expect(defaultLang(["fr-FR", "en-GB", "ja"])).toBe("en");
  expect(defaultLang(["de-DE"])).toBe("en");
  expect(defaultLang([])).toBe("en");
});

test("テーマは data-theme、言語は lang で文書に当てる", () => {
  const root = document.createElement("html");
  applyPrefs("dark", "en", root);
  expect(root.dataset.theme).toBe("dark");
  expect(root.lang).toBe("en");
  // OS に合わせるときは data-theme を付けない（tokens.css が OS の設定で選ぶ）
  applyPrefs("system", "ja", root);
  expect(root.hasAttribute("data-theme")).toBe(false);
  expect(root.lang).toBe("ja");
});

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
