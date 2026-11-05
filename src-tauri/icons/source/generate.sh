#!/bin/sh
# アプリアイコンの作り直し（ビルド・配布手順書 2章「アプリアイコン」）。
# リポジトリ直下で `sh src-tauri/icons/source/generate.sh` を実行する。
# 必要なもの：rsvg-convert（SVG を PNG にする）と pnpm（`pnpm tauri icon` を使う）。
# src-tauri/icons/source/ の SVG から PNG を作り、Tauri で 3 OS 用を生成して src-tauri/icons/ に置く。
set -eu

src=src-tauri/icons/source
out=src-tauri/icons
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

for name in app-icon app-icon-macos; do
  rsvg-convert -w 1024 -h 1024 "$src/$name.svg" -o "$src/$name.png"
done

# Windows・Linux は全面版、macOS は余白つきの版から作る。
pnpm tauri icon "$src/app-icon.png" -o "$tmp/win"
pnpm tauri icon "$src/app-icon-macos.png" -o "$tmp/mac"

# tauri.conf.json の bundle.icon が使うものだけを置く（Android・iOS・ストア用は置かない）。
for f in icon.png 32x32.png 128x128.png 128x128@2x.png icon.ico; do
  cp "$tmp/win/$f" "$out/$f"
done
cp "$tmp/mac/icon.icns" "$out/icon.icns"
echo "作り直しました。docs/release/08-test-plan.md 4.10 のとおり見た目を確かめてください。"
