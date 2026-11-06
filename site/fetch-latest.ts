// いま公開中の latest.json を取って、ファイルに書く。サイトだけの変更のデプロイが、これを含める
// （docs/release/07-build-and-distribution.md 3.4。取れなければ中止する。含め忘れて 404 にしない、古い写しで上書きしない）。
//
//   node site/fetch-latest.ts --out <書き出すパス> [--url <URL>] [--allow-missing]
//
// 終了コード：0 = 取れて書いた、または `--allow-missing` で「まだ公開していない（404）」を許した、
// 1 = 取れなかった（中止）。`--allow-missing` は、最初の公開（latest.json がまだ無い）の手動実行だけで使う。
// 404 以外の失敗（5xx・通信の失敗・形の誤り）は `--allow-missing` でも中止する。
import { writeFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { SITE } from "./lib/consts.ts";
import { fetchPublishedLatest } from "./lib/latest.ts";

async function main(): Promise<number> {
  const { values } = parseArgs({
    options: { out: { type: "string" }, url: { type: "string" }, "allow-missing": { type: "boolean" } },
    allowPositionals: false,
  });
  if (values.out === undefined) throw new Error("--out が要る");
  const url = values.url ?? `${SITE}/latest.json`;
  const allowMissing = values["allow-missing"] === true;

  // キャッシュを避けるため、毎回違う問い合わせ文字列を付ける
  const result = await fetchPublishedLatest(`${url}?_=${Date.now()}`);
  if (result.kind === "ok") {
    writeFileSync(values.out, result.text);
    console.log(`公開中の latest.json（version ${result.latest.version}）を ${values.out} に書いた`);
    return 0;
  }
  // 許すのは「まだ公開していない（404）」だけ。5xx・通信の失敗・形の誤りは、許さずに中止する
  // （壊れた公開物や一時的な障害のときに、latest.json の無いサイトを上げて 404 にしないため）
  if (allowMissing && result.kind === "missing") {
    console.log(`::warning::latest.json を含めずに進める（--allow-missing）: ${result.reason}`);
    return 0;
  }
  console.error(
    `::error::公開中の latest.json を取れないので中止する: ${result.reason}\n` +
      (result.kind === "missing"
        ? "最初の公開（まだ latest.json が無い）なら、手動実行で allow_missing_latest を true にする。"
        : "取得の失敗や形の誤りは、allow_missing_latest でも進めない。原因を直すか、つなぎ直して再実行する。"),
  );
  return 1;
}

try {
  process.exitCode = await main();
} catch (e) {
  console.error(`::error::${e instanceof Error ? e.message : String(e)}`);
  process.exitCode = 1;
}
