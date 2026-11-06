// 公式サイトをビルドする：`pnpm site:build [--out <出力先>] [--latest-json <パス>]`。
// 出力先の既定は site/dist/（コミットしない）。--latest-json を付けると、その latest.json を含める。
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";
import { build, ROOT } from "./lib/build.ts";

function main(): void {
  const { values } = parseArgs({
    options: { out: { type: "string" }, "latest-json": { type: "string" } },
    allowPositionals: false,
  });
  const outDir = values.out !== undefined ? resolve(values.out) : join(ROOT, "site", "dist");
  const latestJson = values["latest-json"] !== undefined ? resolve(values["latest-json"]) : undefined;
  const { latest, files } = build({ outDir, latestJson });
  console.log(`サイトを ${outDir} に書いた（${files.length} ファイル。latest.json: ${latest ? latest.version : "含めない"}）`);
}

main();
