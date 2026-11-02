#!/usr/bin/env python3
"""ビルド済みの成果物から、サイズとコード量を数える。

使い方（experiments/yun-2-frontend で）: python3 scripts/static_metrics.py
"""
import gzip
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APPS = {"svelte": "yun2-svelte", "solid": "yun2-solid"}


def kb(n):
    return round(n / 1024, 1)


def gz(path):
    return len(gzip.compress(path.read_bytes(), 9))


def code_lines(path):
    """空行とコメントだけの行を除いた行数"""
    n = 0
    in_block = False
    for line in path.read_text().splitlines():
        s = line.strip()
        if in_block:
            if "*/" in s or "-->" in s:
                in_block = False
            continue
        if not s or s.startswith("//"):
            continue
        if s.startswith("/*") or s.startswith("<!--"):
            if "*/" not in s and "-->" not in s:
                in_block = True
            continue
        n += 1
    return n


def du(path):
    out = subprocess.run(["du", "-sb", str(path)], capture_output=True, text=True).stdout
    return int(out.split()[0]) if out else 0


def main():
    res = {}
    for app, bin_name in APPS.items():
        a = ROOT / app
        r = {}
        # JS / CSS
        assets = sorted((a / "dist/assets").iterdir())
        r["chunks"] = {
            re.sub(r"-[A-Za-z0-9_-]{8}\.", ".", p.name): {"raw_kb": kb(p.stat().st_size), "gzip_kb": kb(gz(p))}
            for p in assets
        }
        js = [p for p in assets if p.suffix == ".js"]
        r["js_total"] = {"raw_kb": kb(sum(p.stat().st_size for p in js)), "gzip_kb": kb(sum(gz(p) for p in js))}
        # アプリ
        binary = a / "src-tauri/target/release" / bin_name
        r["binary_kb"] = kb(binary.stat().st_size)
        debs = list((a / "src-tauri/target/release/bundle/deb").glob("*.deb"))
        r["deb_kb"] = kb(debs[0].stat().st_size) if debs else None
        # 依存
        lock = (a / "pnpm-lock.yaml").read_text()
        snap = lock.split("\nsnapshots:\n", 1)[1] if "\nsnapshots:\n" in lock else ""
        r["npm_packages"] = len(re.findall(r"^  \S.*:$", snap, re.M))
        r["node_modules_mb"] = round(du(a / "node_modules") / 1024 / 1024, 1)
        # コード量（フレームワーク固有のファイルだけ。shared/ のコピーは除く）
        files = [p for p in (a / "src").rglob("*") if p.is_file() and "shared" not in p.parts]
        files += [a / "vite.config.ts", a / "index.html"] + ([a / "tsconfig.json"] if (a / "tsconfig.json").exists() else [])
        r["code"] = {str(p.relative_to(a)): code_lines(p) for p in files}
        r["code_total"] = sum(r["code"].values())
        res[app] = r
    # 共通コードが両版で同じか
    same = all(
        (ROOT / "svelte/src/shared" / p.name).read_bytes() == (ROOT / "solid/src/shared" / p.name).read_bytes()
        == p.read_bytes()
        for p in (ROOT / "shared").iterdir()
    )
    res["shared_identical"] = same
    res["shared_code_lines"] = {p.name: code_lines(p) for p in sorted((ROOT / "shared").iterdir())}
    out = ROOT / "results/raw/static.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(res, ensure_ascii=False, indent=2))
    print(json.dumps(res, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
