#!/usr/bin/env bash
# shared/ の共通ファイルを、両版の src/shared/ にコピーする。
# 比較を公平にするため、IME 判定・エディタ・計測・スタイルは両版でまったく同じものを使う。
set -euo pipefail
cd "$(dirname "$0")/.."
for app in svelte solid; do
  mkdir -p "$app/src/shared"
  cp shared/*.ts shared/*.css "$app/src/shared/"
done
echo "shared/ を svelte/src/shared と solid/src/shared にコピーしました"
