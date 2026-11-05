// 依存の既知脆弱性の検査（配布用 CI の検査ジョブ。docs/release/07-build-and-distribution.md 3章）。
//
//   node scripts/release/audit.ts rust   … cargo audit（Cargo.lock）
//   node scripts/release/audit.ts js     … pnpm audit（pnpm-lock.yaml。--prod は付けない）
//
// 終了コード：0 = 指摘なし、1 = 脆弱性の指摘がある、2 = 検査できなかった。
// 脆弱性による中止と、データベースやレジストリに届かず検査できなかったことを区別するため、
// 道具の出力（JSON）を読んで分ける。許可した指摘は、道具の設定（.cargo/audit.toml・pnpm-workspace.yaml）で
// 除かれる（scripts/release/allowlist.ts が設定と audit-allowlist.json の一致を確かめる）。
import { spawnSync } from "node:child_process";

export type Outcome =
  | { kind: "ok"; notes: string[] }
  | { kind: "vulnerable"; findings: string[]; notes: string[] }
  | { kind: "unavailable"; reason: string };

type Run = { status: number | null; stdout: string; stderr: string };

function tryParse(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

function lastLines(text: string, n = 8): string {
  return text.trim().split("\n").slice(-n).join("\n");
}

type CargoItem = {
  kind?: string;
  package?: { name?: string; version?: string };
  advisory?: { id?: string; title?: string } | null;
};

/**
 * cargo audit --json の結果を分ける。脆弱性（vulnerabilities）は中止、メンテナンス終了・取り下げ（yanked）・
 * 健全性の問題（unsound）の警告は、中止せず notes に返す（07 3章）。
 */
export function classifyCargoAudit(run: Run): Outcome {
  const json = tryParse(run.stdout) as
    | { vulnerabilities?: { found?: boolean; list?: CargoItem[] }; warnings?: Record<string, CargoItem[]> }
    | undefined;
  if (!json || typeof json !== "object" || !json.vulnerabilities || typeof json.vulnerabilities.found !== "boolean") {
    return { kind: "unavailable", reason: `cargo audit が結果を返さなかった（終了コード ${run.status}）\n${lastLines(run.stderr)}` };
  }
  const describe = (it: CargoItem) =>
    `${it.package?.name}@${it.package?.version} ${it.advisory?.id ?? "(ID なし)"} ${it.advisory?.title ?? ""}`.trim();
  const notes: string[] = [];
  for (const [kind, items] of Object.entries(json.warnings ?? {})) {
    for (const it of items) notes.push(`${it.kind ?? kind}: ${describe(it)}`);
  }
  if (json.vulnerabilities.found) {
    return { kind: "vulnerable", findings: (json.vulnerabilities.list ?? []).map(describe), notes };
  }
  // 指摘なしと言うのは、道具が成功したときだけ（結果の形が読めても、終了コードが 0 でなければ信用しない）
  if (run.status !== 0) {
    return { kind: "unavailable", reason: `cargo audit の終了コードが ${run.status}\n${lastLines(run.stderr)}` };
  }
  return { kind: "ok", notes };
}

/** pnpm audit --json の結果を分ける。重大度のしきい値は付けず、1 件でもあれば中止する（07 3章） */
export function classifyPnpmAudit(run: Run): Outcome {
  const json = tryParse(run.stdout) as
    | {
        metadata?: unknown;
        advisories?: Record<string, { github_advisory_id?: string; module_name?: string; title?: string; severity?: string }>;
      }
    | undefined;
  // `advisories` は許可リスト（pnpm-workspace.yaml）で除いた後の指摘。`metadata.vulnerabilities` の件数は除く前の
  // 件数なので、判定には使わない（許可した指摘を数えてしまう）
  if (!json || typeof json !== "object" || !json.metadata || !json.advisories || typeof json.advisories !== "object") {
    return { kind: "unavailable", reason: `pnpm audit が結果を返さなかった（終了コード ${run.status}）\n${lastLines(run.stderr || run.stdout)}` };
  }
  const found = Object.values(json.advisories);
  if (found.length > 0) {
    const findings = found.map(
      (a) => `${a.module_name} ${a.github_advisory_id ?? ""} ${a.severity ?? ""} ${a.title ?? ""}`.trim(),
    );
    return { kind: "vulnerable", findings, notes: [] };
  }
  if (run.status !== 0) {
    return { kind: "unavailable", reason: `pnpm audit の終了コードが ${run.status}\n${lastLines(run.stderr)}` };
  }
  return { kind: "ok", notes: [] };
}

function run(command: string, args: string[]): Run {
  const r = spawnSync(command, args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024, shell: process.platform === "win32" });
  return { status: r.status, stdout: r.stdout ?? "", stderr: r.error ? String(r.error) : (r.stderr ?? "") };
}

/** 結果を表示して、終了コードを返す */
export function report(name: string, outcome: Outcome): number {
  if (outcome.kind === "ok") {
    for (const n of outcome.notes) console.log(`::warning title=${name}（中止しない警告）::${n}`);
    console.log(`${name}: 指摘なし（警告 ${outcome.notes.length} 件は中止しない）`);
    return 0;
  }
  if (outcome.kind === "vulnerable") {
    for (const n of outcome.notes) console.log(`::warning title=${name}（中止しない警告）::${n}`);
    for (const f of outcome.findings) console.error(`::error title=${name}: 脆弱性の指摘::${f}`);
    console.error(`${name}: 脆弱性の指摘が ${outcome.findings.length} 件ある。依存を上げるか、修正版が無ければ許可リストに載せる（07 3章）`);
    return 1;
  }
  console.error(`::error title=${name}: 検査できなかった::${outcome.reason.split("\n")[0]}`);
  console.error(`${name}: 検査できなかった（脆弱性の指摘ではない）。つなぎ直してから再実行する\n${outcome.reason}`);
  return 2;
}

if (import.meta.main) {
  const which = process.argv[2];
  if (which === "rust") {
    process.exit(report("cargo audit", classifyCargoAudit(run("cargo", ["audit", "--json"]))));
  } else if (which === "js") {
    process.exit(report("pnpm audit", classifyPnpmAudit(run("pnpm", ["audit", "--json"]))));
  } else {
    console.error("使い方: node scripts/release/audit.ts rust|js");
    process.exit(64);
  }
}
