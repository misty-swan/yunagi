// @vitest-environment node
// アプリアイコンの元画像と生成物が、ロゴ（Logo.svelte）と食い違っていないことを確かめる
// （ビルド・配布手順書 2章「アプリアイコン」）。絵そのものは目で見て確かめる。
// 置き場所は install/（ファイルを読むので、Node の型が使えるここに置く）。
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const icons = resolve(root, "src-tauri/icons");
const read = (path: string) => readFileSync(path, "utf8");
const colors = (text: string) =>
  [...text.matchAll(/#[0-9A-Fa-f]{6}|rgba\([^)]*\)/g)].map((m) => m[0].toLowerCase());

/** PNG の先頭（IHDR）から幅と高さを読む。 */
function pngSize(path: string): [number, number] {
  const b = readFileSync(path);
  return [b.readUInt32BE(16), b.readUInt32BE(20)];
}

describe("アプリアイコン", () => {
  const logo = colors(read(resolve(root, "src/shell/Logo.svelte")));
  for (const name of ["app-icon.svg", "app-icon-macos.svg"]) {
    it(`${name} はロゴと同じ色を、同じ順で使う`, () => {
      expect(colors(read(resolve(icons, "source", name)))).toEqual(logo);
    });
  }

  it("macOS 版は全面版と同じ絵を、約 10% の余白を残して縮めたものである", () => {
    const full = read(resolve(icons, "source/app-icon.svg"));
    const mac = read(resolve(icons, "source/app-icon-macos.svg"));
    const inner = (s: string) => s.slice(s.indexOf('<g clip-path="url(#round)">'), s.lastIndexOf("</g>"));
    expect(inner(mac).replace(/\s+/g, "")).toContain(inner(full).replace(/\s+/g, "").replace(/<\/g>$/, ""));
    expect(mac).toContain('translate(100 100) scale(0.8047)');
  });

  it("元の PNG と生成した PNG の大きさが合っている", () => {
    expect(pngSize(resolve(icons, "source/app-icon.png"))).toEqual([1024, 1024]);
    expect(pngSize(resolve(icons, "source/app-icon-macos.png"))).toEqual([1024, 1024]);
    expect(pngSize(resolve(icons, "icon.png"))).toEqual([512, 512]);
    expect(pngSize(resolve(icons, "128x128@2x.png"))).toEqual([256, 256]);
    expect(pngSize(resolve(icons, "128x128.png"))).toEqual([128, 128]);
    expect(pngSize(resolve(icons, "32x32.png"))).toEqual([32, 32]);
  });

  it("icon.ico は 16〜256px を含み、icon.icns は macOS 形式である", () => {
    const ico = readFileSync(resolve(icons, "icon.ico"));
    expect(ico.readUInt16LE(2)).toBe(1); // 種類：アイコン
    expect(ico.readUInt16LE(4)).toBeGreaterThanOrEqual(6); // 絵の数
    expect(readFileSync(resolve(icons, "icon.icns")).subarray(0, 4).toString("ascii")).toBe("icns");
  });
});
