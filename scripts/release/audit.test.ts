// 脆弱性の検査の結果の分け方の試験（docs/release/07-build-and-distribution.md 3章）。
// 脆弱性による中止と、検査できなかったことの区別、警告（メンテナンス終了・yanked・unsound）を中止にしないこと。
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { afterAll, beforeAll, expect, test, vi } from "vitest";
import { classifyCargoAudit, classifyPnpmAudit, report } from "./audit.ts";

// report() は GitHub Actions の注釈（`::warning`・`::error`）を出す。試験の出力が、開発用 CI の注釈として
// 本物の警告のように見えないよう、この試験の中では出力を止める
beforeAll(() => {
  vi.spyOn(console, "log").mockImplementation(() => {});
  vi.spyOn(console, "error").mockImplementation(() => {});
});
afterAll(() => vi.restoreAllMocks());

const pkg = (name: string, version: string) => ({ name, version });
const item = (kind: string, name: string, id: string | null, title = "題名") => ({
  kind,
  package: pkg(name, "1.0.0"),
  advisory: id ? { id, title } : null,
});

const cargoJson = (found: boolean, vulns: object[] = [], warnings: object = {}) =>
  JSON.stringify({ database: {}, lockfile: {}, settings: {}, vulnerabilities: { found, count: vulns.length, list: vulns }, warnings });

test("cargo audit：指摘が無ければ ok。警告は中止せず notes に返す", () => {
  const warnings = {
    unmaintained: [item("unmaintained", "a", "RUSTSEC-2024-0370")],
    unsound: [item("unsound", "glib", "RUSTSEC-2024-0429")],
    yanked: [item("yanked", "clipboard-win", null)],
  };
  const r = classifyCargoAudit({ status: 0, stdout: cargoJson(false, [], warnings), stderr: "" });
  expect(r.kind).toBe("ok");
  if (r.kind !== "ok") return;
  expect(r.notes).toHaveLength(3);
  expect(r.notes.join("\n")).toContain("unsound: glib@1.0.0 RUSTSEC-2024-0429");
  expect(report("cargo audit", r)).toBe(0);
});

test("cargo audit：脆弱性があれば vulnerable（終了コード 1）。終了コードが 0 でも結果の脆弱性を見る", () => {
  const vuln = { package: pkg("zip", "4.6.1"), advisory: { id: "RUSTSEC-2026-0367", title: "SUID" } };
  for (const status of [1, 0]) {
    const r = classifyCargoAudit({ status, stdout: cargoJson(true, [vuln]), stderr: "" });
    expect(r.kind).toBe("vulnerable");
    if (r.kind === "vulnerable") expect(r.findings[0]).toContain("zip@4.6.1 RUSTSEC-2026-0367");
    expect(report("cargo audit", r)).toBe(1);
  }
});

test("cargo audit：データベースに届かない・結果が読めないときは unavailable（終了コード 2）で、脆弱性とは別", () => {
  const stderr = "error: couldn't fetch advisory database: network is down";
  for (const stdout of ["", "not json", "{}", '{"vulnerabilities":{}}']) {
    const r = classifyCargoAudit({ status: 1, stdout, stderr });
    expect(r.kind).toBe("unavailable");
    if (r.kind === "unavailable") expect(r.reason).toContain("network is down");
    expect(report("cargo audit", r)).toBe(2);
  }
  // 結果は読めても、道具が失敗して終わったなら、指摘なしとは言わない
  expect(classifyCargoAudit({ status: 1, stdout: cargoJson(false), stderr }).kind).toBe("unavailable");
});

const pnpmJson = (advisories: object, low = 0) =>
  JSON.stringify({ advisories, metadata: { vulnerabilities: { info: 0, low, moderate: 0, high: 0, critical: 0 } } });

test("pnpm audit：許可リストで除かれた後の指摘（advisories）で判定する。件数（metadata）は使わない", () => {
  // 許可した指摘は metadata の件数には残るが、advisories からは除かれる
  const allowed = classifyPnpmAudit({ status: 0, stdout: pnpmJson({}, 1), stderr: "" });
  expect(allowed.kind).toBe("ok");
  expect(report("pnpm audit", allowed)).toBe(0);

  const advisories = { "1": { module_name: "left-pad", github_advisory_id: "GHSA-aaaa-bbbb-cccc", severity: "low", title: "t" } };
  const r = classifyPnpmAudit({ status: 1, stdout: pnpmJson(advisories, 1), stderr: "" });
  expect(r.kind).toBe("vulnerable");
  if (r.kind === "vulnerable") expect(r.findings[0]).toContain("left-pad GHSA-aaaa-bbbb-cccc low");
  expect(report("pnpm audit", r)).toBe(1);
});

test("pnpm audit：レジストリに届かないときは unavailable（終了コード 2）。結果が読めても終了コードが 0 でなければ信用しない", () => {
  const stderr = "Error: ERR_PNPM_AUDIT_BAD_RESPONSE\nconnection refused";
  for (const stdout of ["", "Error: x", "{}"]) {
    const r = classifyPnpmAudit({ status: 1, stdout, stderr });
    expect(r.kind).toBe("unavailable");
    expect(report("pnpm audit", r)).toBe(2);
  }
  expect(classifyPnpmAudit({ status: 1, stdout: pnpmJson({}), stderr }).kind).toBe("unavailable");
});

// 偽の cargo・pnpm を PATH の先頭に置き、実際のコマンドの呼び出し（引数・終了コード）まで通して確かめる
const dir = mkdtempSync(join(tmpdir(), "yunagi-audit-"));
afterAll(() => rmSync(dir, { recursive: true, force: true }));

function fake(name: string, stdout: string, status: number, stderr = ""): void {
  mkdirSync(dir, { recursive: true });
  const body = `#!/bin/sh\necho "$@" > "${dir}/${name}.args"\nprintf '%s' '${stdout}'\nprintf '%s' '${stderr}' >&2\nexit ${status}\n`;
  writeFileSync(join(dir, name), body);
  chmodSync(join(dir, name), 0o755);
}

function runCli(which: string): { status: number | null; out: string } {
  const r = spawnSync(process.execPath, ["scripts/release/audit.ts", which], {
    encoding: "utf8",
    env: { ...process.env, PATH: `${dir}:${process.env.PATH}` },
  });
  return { status: r.status, out: r.stdout + r.stderr };
}

test.skipIf(process.platform === "win32")("CLI：終了コードは 0（指摘なし）・1（脆弱性）・2（検査できなかった）で、--prod を付けない", () => {
  fake("cargo", cargoJson(false), 0);
  expect(runCli("rust").status).toBe(0);

  fake("cargo", cargoJson(true, [{ package: pkg("zip", "4.6.1"), advisory: { id: "RUSTSEC-2026-0367", title: "x" } }]), 1);
  expect(runCli("rust").status).toBe(1);

  fake("cargo", "", 1, "network is down");
  const unavailable = runCli("rust");
  expect(unavailable.status).toBe(2);
  expect(unavailable.out).toContain("検査できなかった");

  fake("pnpm", pnpmJson({}, 1), 0);
  expect(runCli("js").status).toBe(0);
  expect(spawnSync("cat", [join(dir, "pnpm.args")], { encoding: "utf8" }).stdout.trim()).toBe("audit --json");

  fake("pnpm", "", 1, "ERR_PNPM_AUDIT_BAD_RESPONSE");
  expect(runCli("js").status).toBe(2);

  expect(runCli("other").status).toBe(64);
});

test("TOML の配列の読み取り：行末のコメントの中の文字列・括弧は読まない", async () => {
  const { readStringArray } = await import("./toml.ts");
  const toml = `ignore = [
    "A-1", # 理由は "foo" を見る ]
    "B-2",
]
other = ["C-3"]`;
  expect(readStringArray(toml, "ignore")).toEqual(["A-1", "B-2"]);
  expect(readStringArray(toml, "other")).toEqual(["C-3"]);
  expect(readStringArray(toml, "ignore-build-dependencies")).toEqual([]);
});
