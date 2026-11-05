// 許可外ライセンスの検出の試験（docs/legal/third-party-licenses.md 2章、docs/release/07-build-and-distribution.md 3章）。
// cargo-about・pnpm を動かす部分は配布用 CI で動くので、ここでは判定・設定・フォントの確認を確かめる。
import { cpSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "vitest";
import { type Policy, checkFonts, readAccepted, satisfies, strictAboutToml } from "./licenses.ts";

const allowed = ["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "MPL-2.0"];

test("ライセンス式：OR はどれか 1 つ、AND はすべてが許可に入っていれば満たす", () => {
  expect(satisfies("MIT", allowed)).toBe(true);
  expect(satisfies("Apache-2.0 OR MIT", allowed)).toBe(true);
  expect(satisfies("MIT OR GPL-3.0-only", allowed)).toBe(true);
  expect(satisfies("GPL-3.0-only OR LGPL-2.1-or-later", allowed)).toBe(false);
  expect(satisfies("MIT AND Unicode-3.0", allowed)).toBe(false);
  expect(satisfies("MIT AND (Apache-2.0 OR GPL-2.0-only)", allowed)).toBe(true);
  expect(satisfies("(MIT OR Apache-2.0) AND BSD-3-Clause", allowed)).toBe(true);
  expect(satisfies("BSL-1.0", allowed)).toBe(false);
  expect(satisfies("MIT or Apache-2.0", allowed)).toBe(true); // 演算子の大文字小文字は区別しない
});

test("ライセンス式：例外つき（WITH）は別のライセンス。式が壊れていれば例外", () => {
  expect(satisfies("Apache-2.0 WITH LLVM-exception", allowed)).toBe(false);
  expect(satisfies("Apache-2.0 WITH LLVM-exception", [...allowed, "Apache-2.0 WITH LLVM-exception"])).toBe(true);
  expect(() => satisfies("MIT OR", allowed)).toThrow();
  expect(() => satisfies("(MIT", allowed)).toThrow();
  expect(() => satisfies("MIT Apache-2.0", allowed)).toThrow();
});

test("about.toml の accepted は、license-policy.json の allowed と pending を合わせたものと同じ", () => {
  const policy = JSON.parse(readFileSync("scripts/release/license-policy.json", "utf8")) as Policy;
  const accepted = readAccepted(readFileSync("about.toml", "utf8"));
  expect([...accepted].sort()).toEqual([...policy.allowed, ...policy.pending].sort());
  // 07 3章・third-party-licenses.md 2章に書いた許可の一覧が、そのまま allowed に入っている
  expect(policy.allowed).toEqual(allowed);
  expect(policy.allowed.some((id) => policy.pending.includes(id))).toBe(false);
});

test("許可外の検出用の設定は、pending を accepted から除くだけで、ほかは変えない", () => {
  const toml = readFileSync("about.toml", "utf8");
  const policy = JSON.parse(readFileSync("scripts/release/license-policy.json", "utf8")) as Policy;
  const strict = strictAboutToml(toml, policy.pending);
  expect(readAccepted(strict)).toEqual(policy.allowed);
  expect(strict).toContain("ignore-build-dependencies = true");
  expect(strict).toContain("targets = [");
});

test("同梱するフォントの確認：今のフォルダは通り、知らないファイル・フォルダ・足りないライセンス文書を見つける", () => {
  expect(checkFonts()).toEqual([]);
  const dir = mkdtempSync(join(tmpdir(), "yunagi-fonts-"));
  try {
    cpSync("src/assets/fonts", dir, { recursive: true });
    expect(checkFonts(dir)).toEqual([]);

    writeFileSync(join(dir, "inter", "Extra.ttf"), "x");
    writeFileSync(join(dir, "udev-gothic", "LICENSE_BIZUDGothic"), "x"); // 中身の違いは fonts.test.ts の SHA-256 が見つける
    rmSync(join(dir, "udev-gothic", "LICENSE_JetBrainsMono"));
    cpSync(join(dir, "inter"), join(dir, "noto"), { recursive: true });
    const problems = checkFonts(dir).join("\n");
    expect(problems).toContain("inter/Extra.ttf: 知らないファイル");
    expect(problems).toContain("udev-gothic/LICENSE_JetBrainsMono: 無い");
    expect(problems).toContain("noto: デザインガイド 4章に無いフォントのフォルダ");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
