// @vitest-environment node
// 公開リポジトリの表向きのファイルの検査：本体のライセンス（MIT と Apache-2.0 のデュアル）の表記が
// そろっていること、SECURITY.md の報告先が全文（docs/release/SECURITY.md）と一致していること、
// Issue・PR のテンプレートがあること。決まりは docs/legal/third-party-licenses.md 3章
// と docs/release/SECURITY.md の冒頭（YUN-109）。
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (path: string): string => readFileSync(join(ROOT, path), "utf8");

/** TOML の `[名前]` の表の中身（次の `[` で始まる行の手前まで）。複数行の配列の中の `[` は行頭に来ない前提。 */
function tomlSection(toml: string, name: string): string {
  const start = toml.indexOf(`\n[${name}]\n`);
  if (start < 0) throw new Error(`[${name}] が無い`);
  const rest = toml.slice(start + name.length + 4);
  const next = rest.search(/^\[/m);
  return next < 0 ? rest : rest.slice(0, next);
}

/** コードの囲み（```）と行内コード（`…`）を除いた本文。コード中の `a[i](x)` をリンクと読まないため。 */
function withoutCode(text: string): string {
  return text.replace(/^```[\s\S]*?^```/gm, "").replace(/`[^`\n]*`/g, "");
}

const LICENSE = "MIT OR Apache-2.0";
const CRATES = ["crates/core", "crates/api", "crates/cli", "src-tauri"];

describe("本体のライセンス", () => {
  it("LICENSE-MIT と LICENSE-APACHE が直下にあり、著作権者の表記が入っている", () => {
    const mit = read("LICENSE-MIT");
    expect(mit).toMatch(/^MIT License/);
    expect(mit).toContain("Copyright (c) 2026 misty-swan");
    const apache = read("LICENSE-APACHE");
    expect(apache).toContain("Apache License");
    expect(apache).toContain("Version 2.0, January 2004");
    expect(apache).toContain("Copyright 2026 misty-swan");
  });

  it("ワークスペースの license が SPDX の式で、4 つのクレートがそれを使う", () => {
    expect(tomlSection(read("Cargo.toml"), "workspace.package")).toMatch(
      new RegExp(`^license = "${LICENSE}"$`, "m"),
    );
    for (const dir of CRATES) {
      expect(read(`${dir}/Cargo.toml`), dir).toMatch(/^license\.workspace = true$/m);
    }
  });

  it("package.json の license が同じ", () => {
    expect(JSON.parse(read("package.json")).license).toBe(LICENSE);
  });

  it("README にライセンスの節があり、2 つのファイルへリンクしている", () => {
    const readme = read("README.md");
    expect(readme).toMatch(/^## ライセンス$/m);
    expect(readme).toContain("(LICENSE-MIT)");
    expect(readme).toContain("(LICENSE-APACHE)");
  });
});

describe("SECURITY.md", () => {
  const MAIL = "security@getyunagi.com";

  it("報告先のメールが、ルートの SECURITY.md と全文で一致している", () => {
    expect(read("SECURITY.md")).toContain(MAIL);
    expect(read("docs/release/SECURITY.md")).toContain(MAIL);
  });

  it("応答の目標と、サポートする版の決まりが、全文と一致している", () => {
    for (const phrase of ["3 営業日以内", "10 営業日以内", "最新のマイナー版", "例：1.4.x"]) {
      expect(read("SECURITY.md"), phrase).toContain(phrase);
      expect(read("docs/release/SECURITY.md"), phrase).toContain(phrase);
    }
  });

  it("Private vulnerability reporting へ案内し、公開の Issue に書かないよう求めている", () => {
    const text = read("SECURITY.md");
    expect(text).toContain("Report a vulnerability");
    expect(text).toContain("公開の Issue には書かないでください");
  });
});

/**
 * 公開しない文書を移した先の一覧（docs/ からの相対パス）。ビルド・配布手順書 8.3 の手順で
 * 非公開のリポジトリへ移すとき、3 つのパスをここに入れる。入れると、公開側にその文書が無いこと
 * （戻し忘れ）と、docs/README.md の案内の表がその文書を指していないことを、下の試験が確かめる。
 * 移すまでは空で、docs/README.md の表のパスはすべて実在しなければならない。
 */
const PRIVATE_MOVED: string[] = [];
// 移すと決めた 3 文書：legal/terms-of-service.md、legal/privacy-policy.md、legal/tokushoho.md

function markdownFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(join(ROOT, dir), { withFileTypes: true })) {
    const rel = `${dir}/${entry.name}`;
    if (entry.isDirectory()) out.push(...markdownFiles(rel));
    else if (entry.name.endsWith(".md")) out.push(rel);
  }
  return out;
}

describe("文書の参照", () => {
  const files = [
    "README.md",
    "CONTRIBUTING.md",
    "CODE_OF_CONDUCT.md",
    "SECURITY.md",
    "CLAUDE.md",
    ...markdownFiles("docs"),
  ];

  it("Markdown のリンクの相対パスが、実在するファイルを指している", () => {
    const broken: string[] = [];
    for (const file of files) {
      for (const m of withoutCode(read(file)).matchAll(/\]\(([^)\s]+)\)/g)) {
        const target = m[1].split("#")[0];
        if (target === "" || /^[a-z][a-z0-9+.-]*:/i.test(target)) continue;
        if (!existsSync(join(ROOT, dirname(file), target))) broken.push(`${file} → ${m[1]}`);
      }
    }
    expect(broken).toEqual([]);
  });

  it("docs/README.md の案内の表の文書が、実在する（移した文書を除く）", () => {
    const missing: string[] = [];
    for (const m of read("docs/README.md").matchAll(/^\| ([A-Za-z/-]+) \| ([A-Za-z0-9._-]+\.md) \|/gm)) {
      const path = `${m[1]}/${m[2]}`;
      if (PRIVATE_MOVED.includes(path)) continue;
      if (!existsSync(join(ROOT, "docs", path))) missing.push(path);
    }
    expect(missing).toEqual([]);
  });

  it("移した文書は、公開側に無く、docs/README.md の表も指していない", () => {
    const table = read("docs/README.md");
    for (const path of PRIVATE_MOVED) {
      expect(existsSync(join(ROOT, "docs", path)), path).toBe(false);
      expect(table, path).not.toMatch(new RegExp(`^\\| ${path.replace("/", " \\| ")} \\|`, "m"));
    }
  });
});

describe("Actions の許可の一覧", () => {
  it("GitHub 以外の action がすべて、手順書 8.6 の許可の一覧に書いてある", () => {
    const used = new Set<string>();
    for (const wf of ["ci.yml", "release.yml"]) {
      for (const m of read(`.github/workflows/${wf}`).matchAll(/^\s*(?:-\s+)?uses:\s*([^@\s]+)@/gm)) {
        const repo = m[1];
        if (repo.startsWith("./") || repo.startsWith("actions/")) continue;
        used.add(repo);
      }
    }
    expect(used.size).toBeGreaterThan(0);
    const doc = read("docs/release/07-build-and-distribution.md");
    const section = doc.slice(doc.indexOf("### 8.6"), doc.indexOf("### 8.7"));
    for (const repo of used) expect(section, repo).toContain(`${repo}@*`);
  });
});

describe("公開向けの案内", () => {
  it("CONTRIBUTING・行動規範・Issue と PR のテンプレートがある", () => {
    for (const path of [
      "CONTRIBUTING.md",
      "CODE_OF_CONDUCT.md",
      ".github/pull_request_template.md",
      ".github/ISSUE_TEMPLATE/config.yml",
      ".github/ISSUE_TEMPLATE/bug_report.yml",
      ".github/ISSUE_TEMPLATE/feature_request.yml",
    ]) {
      expect(existsSync(join(ROOT, path)), path).toBe(true);
    }
  });

  it("Issue テンプレートは空の Issue を許さず、脆弱性の窓口を案内する", () => {
    const config = read(".github/ISSUE_TEMPLATE/config.yml");
    expect(config).toMatch(/^blank_issues_enabled: false$/m);
    expect(config).toContain("security/advisories/new");
  });

  it("不具合報告の項目が、サポート手順書 2章の項目をすべて含む", () => {
    const bug = read(".github/ISSUE_TEMPLATE/bug_report.yml");
    for (const label of ["Yunagi の版", "OS と版、CPU", "日本語入力の種類", "Vim モード", "再現の手順", "診断情報"]) {
      expect(bug, label).toContain(label);
    }
  });

  it("CONTRIBUTING が、取り込みを保証しないことと、ライセンスの扱いを書いている", () => {
    const text = read("CONTRIBUTING.md");
    expect(text).toContain("取り込みは保証しません");
    expect(text).toContain("MIT と Apache-2.0");
  });
});
