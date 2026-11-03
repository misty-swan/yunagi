// 色のトークン（tokens.css）が、正本のデザインガイド 3章の表と同じ値かを確かめる。
// ダークの値は CSS の都合で2か所（OS に合わせるとき・ダーク固定）に書くので、両方を確かめる。
import { expect, test } from "vitest";
import guide from "../../docs/design/06-design-guide.md?raw";
import css from "./tokens.css?raw";

/** CSS のブロックの中の `--名前: 値;`（空白を除き、小文字にし、`0.5` を `.5` にそろえる）。 */
function tokens(block: string): Map<string, string> {
  const norm = (v: string) => v.replace(/\s+/g, "").toLowerCase().replace(/(^|[(,])0\./g, "$1.");
  return new Map([...block.matchAll(/--([a-z0-9-]+):\s*([^;]+);/g)].map((m) => [m[1], norm(m[2])]));
}

function block(start: string): string {
  const from = css.indexOf(start);
  expect(from, start).toBeGreaterThanOrEqual(0);
  return css.slice(from, css.indexOf("}", from));
}

test("ライト・ダーク（OS に合わせる・固定）の値が、デザインガイド 3章の表と同じ", () => {
  const light = tokens(block(":root {"));
  const darkOs = tokens(block(':root:not([data-theme="light"])'));
  const darkFixed = tokens(block(':root[data-theme="dark"]'));
  const table = guide.slice(guide.indexOf("## 3. 色"), guide.indexOf("## 4. 文字"));
  const rows = [...table.matchAll(/^\| `--([a-z0-9-]+)`(?: \/ `--([a-z0-9-]+)`)? \| ([^|]+) \| ([^|]+) \|/gm)].filter(
    (r) => r[3].includes("#") || r[3].includes("rgba"),
  );
  const expected = { light: new Map<string, string>(), dark: new Map<string, string>() };
  for (const r of rows) {
    const names = [r[1], r[2]].filter(Boolean);
    const split = (v: string) => v.split(" / ").map((x) => tokens(`--x: ${x};`).get("x"));
    const [l, d] = [split(r[3]), split(r[4])];
    names.forEach((n, i) => {
      expected.light.set(n, l[i] ?? "");
      expected.dark.set(n, d[i] ?? "");
    });
  }
  expect(expected.light.size).toBe(26);
  expect(light).toEqual(expected.light);
  expect(darkOs).toEqual(expected.dark);
  expect(darkFixed).toEqual(expected.dark);
});
