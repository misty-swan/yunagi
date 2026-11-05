// @vitest-environment node
// アプリアイコンの元画像と生成物が、ロゴ（Logo.svelte）と食い違っていないことを確かめる
// （ビルド・配布手順書 2章「アプリアイコン」）。絵そのものは目で見て確かめる。
// 置き場所は install/（tsconfig.node.json の型検査の対象で、ファイルを読む試験をここに集めている）。
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const icons = resolve(root, "src-tauri/icons");
const read = (path: string) => readFileSync(path, "utf8");
const all = (text: string, re: RegExp) => [...text.matchAll(re)].map((m) => m[1]);
// ロゴは 24px の絵、アイコンは 1024px の絵
const scale = 1024 / 24;

const logo = read(resolve(root, "src/shell/Logo.svelte"));
const full = read(resolve(icons, "source/app-icon.svg"));
const mac = read(resolve(icons, "source/app-icon-macos.svg"));

/** 絵の部分（`<g clip-path="url(#round)">` から対応する閉じまで）を、空白を詰めて取り出す。 */
const drawing = (s: string) =>
  s.slice(s.indexOf('<g clip-path="url(#round)">'), s.indexOf("</g>")).replace(/\s+/g, "");

/** PNG の先頭（IHDR）から幅と高さを読む。 */
function pngSize(path: string): [number, number] {
  const b = readFileSync(path);
  return [b.readUInt32BE(16), b.readUInt32BE(20)];
}

describe("アプリアイコン：ロゴとの一致", () => {
  for (const [name, svg] of [
    ["app-icon.svg", full],
    ["app-icon-macos.svg", mac],
  ] as const) {
    it(`${name} はロゴと同じ色・グラデーションの位置を使う`, () => {
      expect(all(svg, /(#[0-9A-Fa-f]{6}|rgba\([^)]*\))/g).map((c) => c.toLowerCase())).toEqual(
        all(logo, /(#[0-9A-Fa-f]{6}|rgba\([^)]*\))/g).map((c) => c.toLowerCase()),
      );
      expect(all(svg, /offset="([^"]+)"/g)).toEqual(all(logo, /offset="([^"]+)"/g));
    });

    it(`${name} は水平線・太陽・光の帯・角丸の位置と大きさがロゴを拡大したものである`, () => {
      const n = (s: string, re: RegExp) => Number(re.exec(s)?.[1]);
      const px = (v: number) => Math.round(v * scale);
      expect(n(svg, /<rect width="1024" height="(\d+)" fill="url\(#sky\)"/)).toBe(
        px(n(logo, /<rect width="24" height="([\d.]+)" fill="url\(#\{sky\}\)"/)),
      );
      expect(n(svg, /<circle cx="\d+" cy="(\d+)"/)).toBe(px(n(logo, /<circle cx="12" cy="([\d.]+)"/)));
      expect(n(svg, /<circle [^>]*r="(\d+)"/)).toBe(px(n(logo, /<circle [^>]*r="([\d.]+)"/)));
      expect(n(svg, /<rect width="1024" height="1024" rx="(\d+)"/)).toBe(
        px(n(logo, /<rect width="24" height="24" rx="([\d.]+)"/)),
      );
      expect(n(svg, /<rect x="(\d+)" y="\d+" width="\d+" height="\d+" fill="rgba/)).toBe(
        px(n(logo, /<rect x="([\d.]+)" y="[\d.]+" width="[\d.]+" height="[\d.]+" fill="rgba/)),
      );
    });
  }

  it("macOS 版は全面版の絵を、四方に約 10% の余白を残して縮めただけのものである", () => {
    expect(drawing(mac)).toBe(drawing(full));
    // 一辺 824px（1024px の 80.47%）を中央に置く
    expect(mac).toContain('<g transform="translate(100 100) scale(0.8047)">');
    expect(Math.abs(824 / 1024 - 0.8047)).toBeLessThan(0.0001);
    // 図形は絵の部分のほかに無い（定義 `<defs>` も同じ）
    const defs = (s: string) => s.slice(s.indexOf("<defs>"), s.indexOf("</defs>")).replace(/\s+/g, "");
    expect(defs(mac)).toBe(defs(full));
    const shapes = (s: string) => (s.match(/<(rect|circle|path|ellipse|g)\b/g) ?? []).length;
    expect(shapes(mac)).toBe(shapes(full) + 1); // 縮める `<g transform>` の 1 つだけ多い
  });
});

describe("アプリアイコン：生成物", () => {
  it("元の PNG と生成した PNG の大きさが合っている", () => {
    expect(pngSize(resolve(icons, "source/app-icon.png"))).toEqual([1024, 1024]);
    expect(pngSize(resolve(icons, "source/app-icon-macos.png"))).toEqual([1024, 1024]);
    expect(pngSize(resolve(icons, "icon.png"))).toEqual([512, 512]);
    expect(pngSize(resolve(icons, "128x128@2x.png"))).toEqual([256, 256]);
    expect(pngSize(resolve(icons, "128x128.png"))).toEqual([128, 128]);
    expect(pngSize(resolve(icons, "32x32.png"))).toEqual([32, 32]);
  });

  it("icon.ico は 16・24・32・48・64・256px を含み、icon.icns は macOS 形式である", () => {
    const ico = readFileSync(resolve(icons, "icon.ico"));
    expect(ico.readUInt16LE(2)).toBe(1); // 種類：アイコン
    const count = ico.readUInt16LE(4);
    // 絵の一覧（16 バイトずつ。幅は 256px のとき 0）
    const sizes = Array.from({ length: count }, (_, i) => ico[6 + i * 16] || 256).sort((a, b) => a - b);
    expect(sizes).toEqual([16, 24, 32, 48, 64, 256]);
    expect(readFileSync(resolve(icons, "icon.icns")).subarray(0, 4).toString("ascii")).toBe("icns");
  });
});
