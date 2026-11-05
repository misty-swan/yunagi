// 許可リストの期限・形・設定との食い違いの検査の試験（docs/release/07-build-and-distribution.md 3章「許可リスト」）。
// 日付は固定して試す。実際のリストの期限は `pnpm check:allowlist` が今日の日付で確かめる。
import { readFileSync } from "node:fs";
import { expect, test } from "vitest";
import { type Allowlist, type Entry, checkAllowlist, checkSync, readCargoIgnore, readPnpmIgnore } from "./allowlist.ts";

const entry = (id: string, expires = "2027-02-05"): Entry => ({
  id,
  reason: "理由",
  remove_when: "上がったら外す",
  expires,
});

const list = (rust: Entry[] = [], javascript: Entry[] = []): Allowlist => ({ rust, javascript });

test("期限内の許可リストは問題なし。期限の当日までは通り、翌日から落ちる", () => {
  const l = list([entry("RUSTSEC-2026-0367")], [entry("GHSA-gcqx-w48m-w2h4")]);
  expect(checkAllowlist(l, "2026-11-05")).toEqual([]);
  expect(checkAllowlist(l, "2027-02-05")).toEqual([]);
  const late = checkAllowlist(l, "2027-02-06");
  expect(late).toHaveLength(2);
  expect(late[0]).toContain("RUSTSEC-2026-0367");
  expect(late[0]).toContain("期限 2027-02-05 を過ぎている");
});

test("期限が実在しない日付・形の違い・先すぎる・理由が空・ID の誤り・重複を見つける", () => {
  const bad = (e: Partial<Entry> & { id: string }, today = "2026-11-05") =>
    checkAllowlist(list([{ ...entry(e.id), ...e }]), today);
  expect(bad({ id: "RUSTSEC-2026-0367", expires: "2027-02-30" })[0]).toContain("実在する YYYY-MM-DD");
  expect(bad({ id: "RUSTSEC-2026-0367", expires: "2027/02/05" })[0]).toContain("実在する YYYY-MM-DD");
  expect(bad({ id: "RUSTSEC-2026-0367", expires: "2099-01-01" })[0]).toContain("先すぎる");
  expect(bad({ id: "RUSTSEC-2026-0367", reason: " " })[0]).toContain("理由");
  expect(bad({ id: "RUSTSEC-2026-0367", remove_when: "" })[0]).toContain("外す条件");
  expect(bad({ id: "GHSA-gcqx-w48m-w2h4" })[0]).toContain("ID の形が違う"); // JavaScript の欄に書く ID ではない（rust の欄）
  const dup = checkAllowlist(list([entry("RUSTSEC-2026-0367"), entry("RUSTSEC-2026-0367")]), "2026-11-05");
  expect(dup.join("\n")).toContain("重複");
  expect(checkAllowlist(list(), "昨日")[0]).toContain("今日の日付");
});

test("検査の道具の設定の ID を読む（コメントの ID は読まない）", () => {
  const toml = `[advisories]
ignore = [
    # "RUSTSEC-2000-0001" はコメント
    "RUSTSEC-2026-0367",
    "RUSTSEC-2026-0368", # 末尾のコメント
]`;
  expect(readCargoIgnore(toml)).toEqual(["RUSTSEC-2026-0367", "RUSTSEC-2026-0368"]);
  const yaml = `# コメント
auditConfig:
  ignoreGhsas:
    # - GHSA-xxxx-xxxx-xxxx
    - GHSA-gcqx-w48m-w2h4
other:
  - not-an-id
`;
  expect(readPnpmIgnore(yaml)).toEqual(["GHSA-gcqx-w48m-w2h4"]);
});

test("正本と設定の ID の食い違いを、足りない側・多い側の両方で見つける", () => {
  const l = list([entry("RUSTSEC-2026-0367")], [entry("GHSA-gcqx-w48m-w2h4")]);
  expect(checkSync(l, ["RUSTSEC-2026-0367"], ["GHSA-gcqx-w48m-w2h4"])).toEqual([]);
  const missing = checkSync(l, [], ["GHSA-gcqx-w48m-w2h4"]);
  expect(missing).toHaveLength(1);
  expect(missing[0]).toContain("が .cargo/audit.toml に無い");
  const extra = checkSync(l, ["RUSTSEC-2026-0367", "RUSTSEC-2026-9999"], ["GHSA-gcqx-w48m-w2h4", "GHSA-aaaa-bbbb-cccc"]);
  expect(extra).toHaveLength(2);
  expect(extra.join("\n")).toContain("RUSTSEC-2026-9999 が audit-allowlist.json に無い");
});

test("リポジトリの許可リストは、形・設定との一致が正しく、固定した日付では期限内", () => {
  const actual = JSON.parse(readFileSync("scripts/release/audit-allowlist.json", "utf8")) as Allowlist;
  expect(checkAllowlist(actual, "2026-11-05")).toEqual([]);
  expect(
    checkSync(
      actual,
      readCargoIgnore(readFileSync(".cargo/audit.toml", "utf8")),
      readPnpmIgnore(readFileSync("pnpm-workspace.yaml", "utf8")),
    ),
  ).toEqual([]);
  // 期限切れを検出できること（YUN-97 の完了の条件。実際の日付の確認は `pnpm check:allowlist`）
  expect(checkAllowlist(actual, "2027-02-06").length).toBe(actual.rust.length + actual.javascript.length);
});
