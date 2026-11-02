#!/usr/bin/env bash
# 両版を本番ビルドし、所要時間を results/raw/build-times.txt に記録する。
#   ./scripts/build-timed.sh clean   … target と dist を消してから（初回ビルド相当）
#   ./scripts/build-timed.sh         … 消さずに（2回目以降のビルド相当）
# Rust の依存はあらかじめ cargo fetch しておく（ダウンロード時間を含めないため）。
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p results/raw
mode="${1:-incremental}"
for app in svelte solid; do
  if [[ "$mode" == clean ]]; then rm -rf "$app/src-tauri/target" "$app/dist"; fi
  start=$EPOCHREALTIME
  (cd "$app" && mise exec rust -- pnpm tauri build --bundles deb >"../results/raw/build-$app-$mode.log" 2>&1)
  end=$EPOCHREALTIME
  printf '%s %s %.1f\n' "$app" "$mode" "$(python3 -c "print($end - $start)")" | tee -a results/raw/build-times.txt
done
