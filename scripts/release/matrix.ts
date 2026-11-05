// 配布用 CI のビルドの組み合わせ（OS × アーキテクチャ。07 2章・3章）を、GitHub Actions の matrix の JSON にする。
//
//   node scripts/release/matrix.ts push           … タグの push（全 6 組）
//   node scripts/release/matrix.ts pull_request   … 試運転（公開リポジトリになるまで使えないランナーの組を除く）
//
// `arch` は配布物の名前に使う 2 つだけ（x86_64・arm64。07 2章）。`target` は Rust の target triple。
export type Entry = {
  name: string;
  /** GitHub Actions のランナー */
  runner: string;
  os: "linux" | "macos" | "windows";
  arch: "x86_64" | "arm64";
  target: string;
  /** `tauri build --bundles` に渡すもの */
  bundles: string;
};

export const ALL: Entry[] = [
  { name: "linux-x86_64", runner: "ubuntu-24.04", os: "linux", arch: "x86_64", target: "x86_64-unknown-linux-gnu", bundles: "appimage" },
  { name: "linux-arm64", runner: "ubuntu-24.04-arm", os: "linux", arch: "arm64", target: "aarch64-unknown-linux-gnu", bundles: "appimage" },
  { name: "macos-arm64", runner: "macos-15", os: "macos", arch: "arm64", target: "aarch64-apple-darwin", bundles: "app" },
  { name: "macos-x86_64", runner: "macos-15", os: "macos", arch: "x86_64", target: "x86_64-apple-darwin", bundles: "app" },
  { name: "windows-x86_64", runner: "windows-2025", os: "windows", arch: "x86_64", target: "x86_64-pc-windows-msvc", bundles: "nsis" },
  { name: "windows-arm64", runner: "windows-2025", os: "windows", arch: "arm64", target: "aarch64-pc-windows-msvc", bundles: "nsis" },
];

/** 試運転で除く組：Linux の arm64 は arm のランナー（`ubuntu-24.04-arm`）が要り、公開リポジトリでないと使えない */
export const DRY_RUN_SKIPPED = ["linux-arm64"];

export function matrixFor(event: string): { include: Entry[] } {
  if (event === "push") return { include: ALL };
  return { include: ALL.filter((e) => !DRY_RUN_SKIPPED.includes(e.name)) };
}

if (import.meta.main) {
  console.log(JSON.stringify(matrixFor(process.argv[2] ?? "push")));
}
