// 配布用 CI：Tauri が作ったファイルを、07 2章の配布物の名前と中身にそろえ、`.sha256` を添える。
//
//   node scripts/release/package.ts <os> <arch> <target> <版> <出力先>
//
//   os = linux | macos | windows、arch = x86_64 | arm64、target = Rust の target triple
//
// Tauri がビルドで付ける名前（`amd64`・`aarch64`・`x64` など）は、ここで x86_64・arm64 に付け替える。
// Updater 用の `.sig`・`latest.json`・`SHA256SUMS` は、ここでは作らない（YUN-101）。
import { execOrThrow as run } from "./exec.ts";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export type Os = "linux" | "macos" | "windows";
export type Arch = "x86_64" | "arm64";

/** 配布物のファイル名（07 2章）。`<ver>` は `v` を付けない */
export const names = {
  macApp: (ver: string, arch: Arch) => `Yunagi_${ver}_${arch}.app.tar.gz`,
  appImage: (ver: string, arch: Arch) => `Yunagi_${ver}_${arch}.AppImage`,
  cli: (ver: string, arch: Arch) => `yunagi-cli_${ver}_${arch}.tar.gz`,
  setup: (ver: string, arch: Arch) => `Yunagi_${ver}_${arch}-setup.exe`,
};

/** `.sha256` の中身：`sha256sum` の出力と同じ `<64 桁の 16 進>  <ファイル名>` の 1 行（07 2章） */
export function sha256Line(hash: string, name: string): string {
  return `${hash}  ${name}\n`;
}

function sha256File(path: string): string {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

/** ディレクトリの中で、拡張子が合うファイルがちょうど 1 つあることを確かめて返す */
function only(dir: string, suffix: string): string {
  if (!existsSync(dir)) throw new Error(`${dir} が無い（Tauri のビルドが作っていない）`);
  const found = readdirSync(dir).filter((f) => f.endsWith(suffix));
  if (found.length !== 1) throw new Error(`${dir} に ${suffix} のファイルがちょうど 1 つあるはずが、${found.length} 個ある: ${found.join(", ")}`);
  return join(dir, found[0]);
}

/** 作った配布物の名前を返す。`.sha256` も出力先に作る */
export function packageArtifacts(os: Os, arch: Arch, target: string, ver: string, out: string, root = "."): string[] {
  const release = join(root, "target", target, "release");
  const bundle = join(release, "bundle");
  mkdirSync(out, { recursive: true });
  const made: string[] = [];

  if (os === "macos") {
    // ルートに Yunagi.app を持つ tar.gz。CLI は Yunagi.app/Contents/MacOS/yunagi（src-tauri/release/macos.json）
    const app = join(bundle, "macos", "Yunagi.app");
    if (!existsSync(join(app, "Contents", "MacOS", "yunagi"))) throw new Error(`${app}/Contents/MacOS/yunagi が無い（CLI が入っていない）`);
    const name = names.macApp(ver, arch);
    run("tar", ["-czf", join(out, name), "-C", join(bundle, "macos"), "Yunagi.app"]);
    made.push(name);
  } else if (os === "linux") {
    const appImage = names.appImage(ver, arch);
    copyFileSync(only(join(bundle, "appimage"), ".AppImage"), join(out, appImage));
    made.push(appImage);
    // CLI だけの配布物。中身はルートの実行ファイル yunagi 1 つ（install.sh が使う）
    const cli = names.cli(ver, arch);
    const exe = join(release, "yunagi");
    if (!existsSync(exe)) throw new Error(`${exe} が無い（yunagi-cli をビルドしていない）`);
    run("tar", ["-czf", join(out, cli), "-C", release, "yunagi"]);
    made.push(cli);
  } else {
    // bin\yunagi.exe と Yunagi.exe を含む（src-tauri/release/windows.json）。中身は Windows でしか見られないので、実機で確かめる（08 4.9）
    const setup = names.setup(ver, arch);
    copyFileSync(only(join(bundle, "nsis"), "-setup.exe"), join(out, setup));
    made.push(setup);
  }

  for (const name of [...made]) {
    writeFileSync(join(out, `${name}.sha256`), sha256Line(sha256File(join(out, name)), name));
    made.push(`${name}.sha256`);
  }
  return made;
}

if (import.meta.main) {
  const [os, arch, target, ver, out] = process.argv.slice(2);
  if (!["linux", "macos", "windows"].includes(os) || !["x86_64", "arm64"].includes(arch) || !target || !ver || !out) {
    console.error("使い方: node scripts/release/package.ts <linux|macos|windows> <x86_64|arm64> <target> <版> <出力先>");
    process.exit(64);
  }
  for (const name of packageArtifacts(os as Os, arch as Arch, target, ver, out)) console.log(name);
}
