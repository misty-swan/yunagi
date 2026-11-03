// 同梱するフォントのファイルが、デザインガイド 4章に書いた版のもの（SHA-256 が同じ）で、
// 許諾の表記が同じ場所にあるかを確かめる（配布用のビルドでも確かめる。ビルド・配布手順書 3章）。
import { expect, test } from "vitest";
import guide from "../../docs/design/06-design-guide.md?raw";
import interLicense from "../assets/fonts/inter/LICENSE.txt?raw";
import inter from "../assets/fonts/inter/InterVariable.woff2?inline";

async function sha256(dataUrl: string): Promise<string> {
  const bytes = Uint8Array.from(atob(dataUrl.slice(dataUrl.indexOf(",") + 1)), (c) => c.charCodeAt(0));
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

test("Inter は 06 4章の版のファイルで、許諾の表記がある", async () => {
  const hash = await sha256(inter);
  expect(guide).toContain(`\`web/InterVariable.woff2\``);
  expect(guide).toContain(`SHA-256 は \`${hash}\``);
  expect(hash).toBe("693b77d4f32ee9b8bfc995589b5fad5e99adf2832738661f5402f9978429a8e3");
  expect(interLicense).toContain("Copyright (c) 2016 The Inter Project Authors");
  expect(interLicense).toContain("SIL Open Font License, Version 1.1");
});
