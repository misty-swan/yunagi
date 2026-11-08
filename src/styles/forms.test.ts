// フォーム部品の共通の見た目（forms.css。デザインガイド 11章）の決まりを確かめる。
// 色はトークン経由だけ、動きは付けない、標準の見た目のままの部品を画面に残さない。
import { expect, test } from "vitest";
import forms from "./forms.css?raw";
import tokens from "./tokens.css?raw";

const screens = import.meta.glob("../**/*.svelte", { query: "?raw", import: "default", eager: true }) as Record<
  string,
  string
>;

/** コメントを除いた CSS。 */
const css = forms.replace(/\/\*[\s\S]*?\*\//g, "");

test("色はトークン（var(--…)）と transparent だけで、直接の色の値を書いていない", () => {
  expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  expect(css).not.toMatch(/\b(rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch)\(/i);
  expect(css).not.toMatch(
    /:\s*[^;{}]*\b(white|black|red|green|blue|gray|grey|orange|yellow|purple|pink|brown|cyan|magenta|currentColor)\b[^;{}]*;/i,
  );
});

test("使う変数は、tokens.css か、base.css の寸法（--u・--fs-small）にある", () => {
  // base.css は読み込まない（@font-face の url() をテストで処理させないため）。使ってよい寸法の変数だけ書く
  const defined = new Set([...tokens.matchAll(/--([a-z0-9-]+)\s*:/g)].map((m) => m[1]).concat(["u", "fs-small"]));
  const used = new Set([...css.matchAll(/var\(--([a-z0-9-]+)/g)].map((m) => m[1]));
  expect(used.size).toBeGreaterThan(0);
  for (const name of used) expect(defined.has(name), `--${name}`).toBe(true);
});

test("動き（transition・animation）を付けていない", () => {
  expect(css).not.toMatch(/\b(transition|animation)\b/);
});

test("外部の読み込み（url()・@import）を使っていない", () => {
  expect(css).not.toMatch(/url\(|@import/);
});

test("状態が揃っている：ホバー・フォーカス・押下・無効・選択済み", () => {
  for (const state of [":hover", ":focus-visible", ":active", ":disabled", ":checked"]) {
    expect(css, state).toContain(state);
  }
});

test("標準の選択欄・チェックボックス・ラジオ・スライダー・日付・時刻・数値の欄は、共通の class を付けている", () => {
  const rules: [RegExp, string][] = [
    [/<select\b/, "fc-select"],
    [/<input\b[^>]*type="checkbox"/, "fc-check"],
    [/<input\b[^>]*type="radio"/, "fc-radio"],
    [/<input\b[^>]*type="range"/, "fc-range"],
    [/type="(date|time|number|text)"/, "fc-field"],
  ];
  // 独自の見た目を持つ部品は対象外：持ち越しの丸いチェック、検索パレットの入力欄
  const exempt = [/class="cbox"/, /role="combobox"/];
  let seen = 0;
  for (const [file, src] of Object.entries(screens)) {
    for (const tag of tags(src, "input").concat(tags(src, "select"))) {
      if (exempt.some((re) => re.test(tag))) continue;
      for (const [re, cls] of rules) {
        if (!re.test(tag)) continue;
        seen++;
        expect(tag, `${file}: ${cls}`).toContain(cls);
      }
    }
  }
  expect(seen).toBeGreaterThan(10);
});

/** 開始タグの全体（`=>` や `{…}` の中の `>` で切れないよう、波括弧と引用符を数えて閉じの `>` を探す）。 */
function tags(src: string, name: string): string[] {
  const out: string[] = [];
  for (const m of src.matchAll(new RegExp(`<${name}(?=[\\s>])`, "g"))) {
    let depth = 0;
    let quote = "";
    let i = m.index;
    for (; i < src.length; i++) {
      const c = src[i];
      if (quote) {
        if (c === quote) quote = "";
      } else if (c === '"' && depth === 0) quote = c;
      else if (c === "{") depth++;
      else if (c === "}") depth--;
      else if (c === ">" && depth === 0 && src[i - 1] !== "=") break;
    }
    out.push(src.slice(m.index, i + 1));
  }
  return out;
}
