// 公式サイトをビルドする：`pnpm site:build [--out <出力先>] [--latest-json <パス>]`。
// 出力先の既定は site/dist/（コミットしない）。--latest-json を付けると、その latest.json を含める。
import { join, resolve } from "node:path";
import { build, ROOT } from "./lib/build.ts";

function main(): void {
  const args = process.argv.slice(2);
  let outDir = join(ROOT, "site", "dist");
  let latestJson: string | undefined;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--out" && args[i + 1]) outDir = resolve(args[++i]);
    else if (args[i] === "--latest-json" && args[i + 1]) latestJson = resolve(args[++i]);
    else throw new Error(`知らない引数: ${args[i]}（使えるのは --out <出力先> と --latest-json <パス>）`);
  }
  const { latest, files } = build({ outDir, latestJson });
  console.log(`サイトを ${outDir} に書いた（${files.length} ファイル。latest.json: ${latest ? latest.version : "含めない"}）`);
}

main();
