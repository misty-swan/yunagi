// 許可リストの期限と、設定との食い違いを確かめる（`pnpm check:allowlist`。リポジトリ直下で動かす）。
// 期限を過ぎると、このスクリプトが落ちるので、開発用 CI も配布用 CI も止まる（決まりは
// docs/release/07-build-and-distribution.md 3章「許可リスト」）。
// 日付を決めて試すときは `--today YYYY-MM-DD` を付ける。
import { readFileSync } from "node:fs";
import {
  type Allowlist,
  checkAllowlist,
  checkSync,
  readCargoIgnore,
  readPnpmIgnore,
  todayUtc,
} from "./allowlist.ts";

const args = process.argv.slice(2);
const i = args.indexOf("--today");
const today = i >= 0 ? (args[i + 1] ?? "") : todayUtc();

const list = JSON.parse(readFileSync("scripts/release/audit-allowlist.json", "utf8")) as Allowlist;
const problems = [
  ...checkAllowlist(list, today),
  ...checkSync(
    list,
    readCargoIgnore(readFileSync(".cargo/audit.toml", "utf8")),
    readPnpmIgnore(readFileSync("pnpm-workspace.yaml", "utf8")),
  ),
];

if (problems.length > 0) {
  for (const p of problems) console.error(`許可リスト: ${p}`);
  process.exit(1);
}
console.log(`許可リスト: ${list.rust.length + list.javascript.length} 件、期限内（${today} 時点）`);
