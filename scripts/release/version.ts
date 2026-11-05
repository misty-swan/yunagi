// 配布用 CI の版の確認：タグ `v<ver>` と、直下の Cargo.toml の `[workspace.package]` の `version` が同じことを確かめる。
// 版は 1 か所（Cargo.toml）にだけ書く（README.md）。タグだけ進めて版を上げ忘れると、別の版の中身に新しい版の名前が付く。
//
//   node scripts/release/version.ts            … 版を表示する（タグの確認はしない）
//   node scripts/release/version.ts v1.2.3     … タグが `v<版>` と同じことを確かめて、版を表示する
//
// 終了コード：0 = 一致、1 = 食い違い。表示するのは版（`1.2.3`）だけ。
import { readFileSync } from "node:fs";

export const VERSION_PATTERN = /^\d+\.\d+\.\d+$/;

/** `[workspace.package]` の `version`。無ければ null */
export function workspaceVersion(cargoToml: string): string | null {
  let inSection = false;
  for (const line of cargoToml.split("\n")) {
    const header = /^\[(.+)\]\s*$/.exec(line);
    if (header) {
      inSection = header[1] === "workspace.package";
    } else if (inSection) {
      const m = /^version\s*=\s*"([^"]+)"/.exec(line);
      if (m) return m[1];
    }
  }
  return null;
}

/** タグが配布の版に合うかの問題を返す。無ければ null */
export function checkTag(tag: string, version: string | null): string | null {
  if (version === null) return "Cargo.toml の [workspace.package] に version が無い";
  if (!VERSION_PATTERN.test(version)) return `Cargo.toml の版 ${version} が 1.2.3 の形でない（07 2章）`;
  if (tag !== `v${version}`) return `タグ ${tag} が Cargo.toml の版 v${version} と違う。版を上げてからタグを打つ（README.md）`;
  return null;
}

if (import.meta.main) {
  const version = workspaceVersion(readFileSync("Cargo.toml", "utf8"));
  const tag = process.argv[2];
  // 版は workflow の run: に埋め込まれ、配布物の名前にもなる。タグの確認をしないとき（試運転）も、形を確かめる
  const problem =
    tag === undefined
      ? version === null
        ? "Cargo.toml に版が無い"
        : VERSION_PATTERN.test(version)
          ? null
          : `Cargo.toml の版 ${version} が 1.2.3 の形でない（07 2章）`
      : checkTag(tag, version);
  if (problem) {
    console.error(`::error title=版の確認::${problem}`);
    process.exit(1);
  }
  console.log(version);
}
