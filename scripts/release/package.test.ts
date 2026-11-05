// 配布用ビルドの、版の確認・組み合わせ・配布物の名前と中身の試験（docs/release/07-build-and-distribution.md 2章・3章）。
// 実際の Tauri のビルド（バンドル）は配布用 CI の試運転が通す。ここでは、Tauri の出力を模した一時フォルダで、
// 付け替える名前と、`.sha256` の形、tar.gz の中身を確かめる。
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, expect, test } from "vitest";
import { ALL, DRY_RUN_SKIPPED, matrixFor } from "./matrix.ts";
import { names, packageArtifacts, sha256Line } from "./package.ts";
import { checkTag, workspaceVersion } from "./version.ts";

const cargo = `[workspace]
members = ["a"]

[workspace.package]
version = "1.2.3"
edition = "2024"

[workspace.dependencies]
version = "9.9.9"
`;

test("版：workspace.package の version を読み、タグ v<版> と同じことを確かめる", () => {
  expect(workspaceVersion(cargo)).toBe("1.2.3");
  expect(workspaceVersion('[package]\nversion = "0.1.0"\n')).toBeNull();
  expect(workspaceVersion(readFileSync("Cargo.toml", "utf8"))).toMatch(/^\d+\.\d+\.\d+$/);
  expect(checkTag("v1.2.3", "1.2.3")).toBeNull();
  expect(checkTag("v1.2.4", "1.2.3")).toContain("違う");
  expect(checkTag("1.2.3", "1.2.3")).toContain("違う"); // v が無い
  expect(checkTag("v1.2.3", null)).toContain("version が無い");
  expect(checkTag("v1.2.3-rc1", "1.2.3-rc1")).toContain("1.2.3 の形でない");
});

test("組み合わせ：タグの push は 6 組すべて、試運転は arm の Linux を除く。arch は 2 種類だけ", () => {
  expect(matrixFor("push").include).toHaveLength(6);
  expect(matrixFor("pull_request").include.map((e) => e.name)).not.toContain("linux-arm64");
  expect(matrixFor("pull_request").include).toHaveLength(6 - DRY_RUN_SKIPPED.length);
  expect(new Set(ALL.map((e) => e.arch))).toEqual(new Set(["x86_64", "arm64"]));
  // テスト計画書 3章・07 2章の対象と 1 対 1：OS ごとに x86_64 と arm64
  for (const os of ["linux", "macos", "windows"]) expect(ALL.filter((e) => e.os === os).map((e) => e.arch).sort()).toEqual(["arm64", "x86_64"]);
  expect(new Set(ALL.map((e) => e.name)).size).toBe(ALL.length);
});

test("version.ts・matrix.ts の CLI：版の不一致と形の誤りは終了コード 1、matrix は JSON を出す", () => {
  const dir = mkdtempSync(join(tmpdir(), "yunagi-version-"));
  const node = (args: string[], cwd = ".") => spawnSync(process.execPath, args, { encoding: "utf8", cwd });
  try {
    const script = join(process.cwd(), "scripts/release/version.ts");
    writeFileSync(join(dir, "Cargo.toml"), cargo);
    expect(node([script], dir).stdout.trim()).toBe("1.2.3");
    expect(node([script, "v1.2.3"], dir).status).toBe(0);
    const mismatch = node([script, "v9.9.9"], dir);
    expect(mismatch.status).toBe(1);
    expect(mismatch.stdout + mismatch.stderr).toContain("違う");
    // 試運転（タグ無し）でも、版の形が誤っていれば止める（版は workflow の run: に埋め込まれる）
    writeFileSync(join(dir, "Cargo.toml"), cargo.replace("1.2.3", "1.0.0$(echo x)"));
    expect(node([script], dir).status).toBe(1);
    writeFileSync(join(dir, "Cargo.toml"), "[workspace]\n");
    expect(node([script], dir).status).toBe(1);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  const push = JSON.parse(node(["scripts/release/matrix.ts", "push"]).stdout);
  const dry = JSON.parse(node(["scripts/release/matrix.ts", "pull_request"]).stdout);
  expect(push.include).toHaveLength(6);
  expect(dry.include.map((e: { name: string }) => e.name)).not.toContain("linux-arm64");
});

test("配布物の名前は 07 2章のとおり（v を付けない・arch は x86_64 か arm64）", () => {
  expect(names.macApp("1.2.3", "arm64")).toBe("Yunagi_1.2.3_arm64.app.tar.gz");
  expect(names.appImage("1.2.3", "x86_64")).toBe("Yunagi_1.2.3_x86_64.AppImage");
  expect(names.cli("1.2.3", "arm64")).toBe("yunagi-cli_1.2.3_arm64.tar.gz");
  expect(names.setup("1.2.3", "x86_64")).toBe("Yunagi_1.2.3_x86_64-setup.exe");
  const hash = "a".repeat(64);
  expect(sha256Line(hash, "x.tar.gz")).toBe(`${hash}  x.tar.gz\n`);
});

const root = mkdtempSync(join(tmpdir(), "yunagi-package-"));
afterAll(() => rmSync(root, { recursive: true, force: true }));

function put(path: string, text = "x"): void {
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, text);
}

const tar = (args: string[]) => spawnSync("tar", args, { encoding: "utf8" });
const sha = (path: string) => createHash("sha256").update(readFileSync(path)).digest("hex");

test.skipIf(process.platform === "win32")("Linux：AppImage を付け替え、CLI だけの tar.gz はルートに yunagi 1 つ。.sha256 は sha256sum と同じ形", () => {
  const t = "aarch64-unknown-linux-gnu";
  put(join(root, "linux", "target", t, "release", "bundle", "appimage", "Yunagi_1.2.3_aarch64.AppImage"), "appimage");
  put(join(root, "linux", "target", t, "release", "yunagi"), "cli");
  const out = join(root, "linux", "out");
  const made = packageArtifacts("linux", "arm64", t, "1.2.3", out, join(root, "linux"));
  expect(made.sort()).toEqual([
    "Yunagi_1.2.3_arm64.AppImage",
    "Yunagi_1.2.3_arm64.AppImage.sha256",
    "yunagi-cli_1.2.3_arm64.tar.gz",
    "yunagi-cli_1.2.3_arm64.tar.gz.sha256",
  ]);
  expect(tar(["-tzf", join(out, "yunagi-cli_1.2.3_arm64.tar.gz")]).stdout.trim()).toBe("yunagi");
  for (const name of ["Yunagi_1.2.3_arm64.AppImage", "yunagi-cli_1.2.3_arm64.tar.gz"]) {
    expect(readFileSync(join(out, `${name}.sha256`), "utf8")).toBe(`${sha(join(out, name))}  ${name}\n`);
  }
});

test.skipIf(process.platform === "win32")("macOS：ルートに Yunagi.app、CLI は Contents/MacOS/yunagi。CLI が無ければ失敗する", () => {
  const t = "x86_64-apple-darwin";
  const macos = join(root, "mac", "target", t, "release", "bundle", "macos");
  put(join(macos, "Yunagi.app", "Contents", "MacOS", "yunagi-app"), "app");
  expect(() => packageArtifacts("macos", "x86_64", t, "1.2.3", join(root, "mac", "out"), join(root, "mac"))).toThrow("CLI が入っていない");
  put(join(macos, "Yunagi.app", "Contents", "MacOS", "yunagi"), "cli");
  const out = join(root, "mac", "out");
  expect(packageArtifacts("macos", "x86_64", t, "1.2.3", out, join(root, "mac")).sort()).toEqual([
    "Yunagi_1.2.3_x86_64.app.tar.gz",
    "Yunagi_1.2.3_x86_64.app.tar.gz.sha256",
  ]);
  const list = tar(["-tzf", join(out, "Yunagi_1.2.3_x86_64.app.tar.gz")]).stdout.split("\n");
  expect(list).toContain("Yunagi.app/Contents/MacOS/yunagi");
  expect(list.every((l) => l === "" || l.startsWith("Yunagi.app/"))).toBe(true);
});

test("Windows：-setup.exe を付け替える。Tauri の出力が 0 個・2 個以上なら失敗する", () => {
  const t = "aarch64-pc-windows-msvc";
  const nsis = join(root, "win", "target", t, "release", "bundle", "nsis");
  expect(() => packageArtifacts("windows", "arm64", t, "1.2.3", join(root, "win", "out"), join(root, "win"))).toThrow("が無い");
  put(join(nsis, "Yunagi_1.2.3_arm64-setup.exe"), "setup");
  const out = join(root, "win", "out");
  expect(packageArtifacts("windows", "arm64", t, "1.2.3", out, join(root, "win")).sort()).toEqual([
    "Yunagi_1.2.3_arm64-setup.exe",
    "Yunagi_1.2.3_arm64-setup.exe.sha256",
  ]);
  put(join(nsis, "Yunagi_1.2.3_x64-setup.exe"), "other arch"); // 別の arch の古いファイルは数えない
  expect(packageArtifacts("windows", "arm64", t, "1.2.3", out, join(root, "win")).sort()).toContain("Yunagi_1.2.3_arm64-setup.exe");
  put(join(nsis, "Yunagi_0.9.0_arm64-setup.exe"), "stale");
  expect(() => packageArtifacts("windows", "arm64", t, "1.2.3", out, join(root, "win"))).toThrow("ちょうど 1 つ");
});

test("ビルドの設定：CLI は macOS で Contents/MacOS/yunagi、Windows で bin/yunagi.exe。更新用の成果物はまだ作らない", () => {
  const read = (f: string) => JSON.parse(readFileSync(`src-tauri/release/${f}`, "utf8"));
  expect(read("macos.json").bundle.macOS.files).toEqual({ "MacOS/yunagi": "cli-bin/yunagi" });
  expect(read("windows.json").bundle.resources).toEqual({ "cli-bin/yunagi.exe": "bin/yunagi.exe" });
  // 本体は Yunagi.exe（install.ps1・yunagi show が探す名前）。macOS は CLI の yunagi と大文字小文字だけの違いになるので変えない
  expect(read("windows.json").mainBinaryName).toBe("Yunagi");
  expect(read("macos.json").mainBinaryName).toBeUndefined();
  expect(read("common.json").mainBinaryName).toBeUndefined();
  expect(read("common.json").bundle.active).toBe(true);
  expect(read("common.json").bundle.resources).toEqual({ "THIRD_PARTY_LICENSES.md": "THIRD_PARTY_LICENSES.md" });
  // createUpdaterArtifacts は署名鍵が要る（YUN-100・YUN-101）。有効にするのは、鍵を CI に置く課題
  expect(JSON.stringify([read("common.json"), read("macos.json"), read("windows.json")])).not.toContain("createUpdaterArtifacts");
  // 普段の tauri.conf.json は配布用のビルドの設定を持たない（pnpm tauri dev と窓の無い試験に、無いファイルを要求させない）
  const conf = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
  expect(conf.bundle.active).toBe(false);
  expect(conf.bundle.resources).toBeUndefined();
  expect(conf.bundle.macOS).toBeUndefined();
});
