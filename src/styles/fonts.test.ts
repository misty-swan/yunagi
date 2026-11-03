// 同梱するフォントのファイルが、デザインガイド 4章に書いた版のもの（SHA-256 が同じ）で、
// 許諾の表記が同じ場所にあるかを確かめる（配布用のビルドでも確かめる。ビルド・配布手順書 3章）。
import { expect, test } from "vitest";
import guide from "../../docs/design/06-design-guide.md?raw";
import interLicense from "../assets/fonts/inter/LICENSE.txt?raw";
import inter from "../assets/fonts/inter/InterVariable.woff2?inline";
import udevLicense from "../assets/fonts/udev-gothic/LICENSE?raw";
import bizLicense from "../assets/fonts/udev-gothic/LICENSE_BIZUDGothic?raw";
import jbLicense from "../assets/fonts/udev-gothic/LICENSE_JetBrainsMono?raw";
import udev from "../assets/fonts/udev-gothic/UDEVGothicHS-Regular.ttf?inline";

async function hex(bytes: Uint8Array<ArrayBuffer>): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

const sha256 = (dataUrl: string) =>
  hex(Uint8Array.from(atob(dataUrl.slice(dataUrl.indexOf(",") + 1)), (c) => c.charCodeAt(0)));
const sha256Text = (text: string) => hex(new TextEncoder().encode(text));

test("Inter は 06 4章の版のファイルで、許諾の表記がある", async () => {
  const hash = await sha256(inter);
  expect(guide).toContain(`\`web/InterVariable.woff2\``);
  expect(guide).toContain(`SHA-256 は \`${hash}\``);
  expect(hash).toBe("693b77d4f32ee9b8bfc995589b5fad5e99adf2832738661f5402f9978429a8e3");
  expect(interLicense).toContain("Copyright (c) 2016 The Inter Project Authors");
  expect(interLicense).toContain("SIL Open Font License, Version 1.1");
});

test("UDEV Gothic HS は 06 4章の版のファイルで、3つの許諾の文書が 06 に書いた版のもの", async () => {
  const hash = await sha256(udev);
  expect(hash).toBe("89fa22a7fe28aca7ab3cdf33812406589b2e28739bcdb051848913e4164a2339");
  expect(guide).toContain(`SHA-256 は \`${hash}\``);
  const licenses: [string, string, string][] = [
    ["LICENSE", udevLicense, "3e04c747c9c9535224f6163a290caf022877759db5183a00aef6134d3f5643f0"],
    ["LICENSE_JetBrainsMono", jbLicense, "30f0c136e3c88e422d0791acd97238870f9054a9729bc34cf2ff0d4ed8cac4ad"],
    ["LICENSE_BIZUDGothic", bizLicense, "e753d7155d53c747d037a445e584c8ecfca6dd79846db610417e282a736b28bc"],
  ];
  for (const [name, text, expected] of licenses) {
    const h = await sha256Text(text);
    expect(h, name).toBe(expected);
    expect(guide).toContain(`\`${name}\`（SHA-256 \`${h}\`）`);
  }
  expect(udevLicense).toContain("Copyright (c) 2022 Yuko OTAWARA");
  expect(jbLicense).toContain("Copyright 2020 The JetBrains Mono Project Authors");
  expect(bizLicense).toContain("Copyright 2022 The BIZ UDGothic Project Authors");
  for (const text of [udevLicense, jbLicense, bizLicense]) expect(text).toContain("SIL Open Font License, Version 1.1");
});
