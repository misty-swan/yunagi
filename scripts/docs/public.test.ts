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

  it(".github のテンプレートが指すこのリポジトリのファイル（…/blob/main/<パス>）が実在する", () => {
    const broken: string[] = [];
    const templates = [
      ".github/pull_request_template.md",
      ".github/ISSUE_TEMPLATE/config.yml",
      ".github/ISSUE_TEMPLATE/bug_report.yml",
      ".github/ISSUE_TEMPLATE/feature_request.yml",
    ];
    for (const file of templates) {
      for (const m of read(file).matchAll(/misty-swan\/yunagi\/blob\/main\/([^\s)"']+)/g)) {
        if (!existsSync(join(ROOT, m[1]))) broken.push(`${file} → ${m[1]}`);
      }
      for (const m of withoutCode(read(file)).matchAll(/\]\(([^)\s]+)\)/g)) {
        const target = m[1].split("#")[0];
        if (target === "" || /^[a-z][a-z0-9+.-]*:/i.test(target)) continue;
        if (!existsSync(join(ROOT, target.startsWith("/") ? target.slice(1) : join(dirname(file), target))))
          broken.push(`${file} → ${m[1]}`);
      }
    }
    expect(broken).toEqual([]);
  });

  it("docs/README.md の案内の表の文書が、実在する", () => {
    const missing: string[] = [];
    for (const m of read("docs/README.md").matchAll(/^\| ([A-Za-z/-]+) \| ([A-Za-z0-9._-]+\.md) \|/gm)) {
      const path = `${m[1]}/${m[2]}`;
      if (!existsSync(join(ROOT, "docs", path))) missing.push(path);
    }
    expect(missing).toEqual([]);
  });
});

describe("法務の草案の表示", () => {
  // 弁護士確認前の草案を公開に残すための表示（ビルド・配布手順書 8.3。YUN-110）。確定版（YUN-106）で外すときは、この試験も直す。
  const DRAFT_DOCS = ["terms-of-service.md", "privacy-policy.md", "tokushoho.md"];

  it("3 文書の題名の直後（3 行目と 5 行目）に、日本語と英語の草案の表示が引用ブロックである", () => {
    for (const name of DRAFT_DOCS) {
      const lines = read(`docs/legal/${name}`).split("\n");
      expect(lines[0], name).toMatch(/^# /);
      expect(lines[2], name).toBe("> **【草案】弁護士確認前の草案です。有効なものではありません。確定版は公式サイトに掲載します。**");
      expect(lines[4], name).toMatch(/^> \*DRAFT — not reviewed by a lawyer /);
    }
  });

  it("特定商取引法の表記は、所在地と電話番号が仮置きのまま（個人情報を入れない）", () => {
    const text = read("docs/legal/tokushoho.md");
    expect(text).toMatch(/^\| 所在地 \| 〔住所〕 \|$/m);
    expect(text).toMatch(/^\| 電話番号 \| 〔電話番号〕 \|$/m);
  });

  it("3 文書は引き続き docs/README.md の案内の表にある", () => {
    const table = read("docs/README.md");
    for (const name of DRAFT_DOCS) expect(table, name).toMatch(new RegExp(`^\\| legal \\| ${name} \\|`, "m"));
  });
});

describe("Actions の許可の一覧", () => {
  it("GitHub 以外の action がすべて、手順書 8.6 の許可の一覧に書いてある", () => {
    const used = new Set<string>();
    const workflows = readdirSync(join(ROOT, ".github/workflows")).filter((f) => /\.ya?ml$/.test(f));
    expect(workflows.length).toBeGreaterThan(0);
    for (const wf of workflows) {
      for (const m of read(`.github/workflows/${wf}`).matchAll(/^\s*(?:-\s+)?uses:\s*([^@\s]+)@/gm)) {
        const repo = m[1];
        if (repo.startsWith("./") || repo.startsWith("actions/")) continue;
        used.add(repo);
      }
    }
    expect(used.size).toBeGreaterThan(0);
    const doc = read("docs/release/07-build-and-distribution.md");
    const from = doc.indexOf("### 8.6");
    const to = doc.indexOf("### 8.7");
    expect(from, "8.6 の見出し").toBeGreaterThan(-1);
    expect(to, "8.7 の見出し").toBeGreaterThan(from);
    // 「Allow specified actions and reusable workflows」の欄に書く、`a/b@*, c/d@*` の形の行
    const line = doc
      .slice(from, to)
      .split("\n")
      .find((l) => l.includes("Allow specified actions"));
    expect(line, "許可の一覧の行").toBeDefined();
    const listed = new Set(
      [...(line ?? "").matchAll(/`([^`]*@\*[^`]*)`/g)].flatMap((m) => m[1].split(",").map((s) => s.trim())),
    );
    for (const repo of used) expect(listed, repo).toContain(`${repo}@*`);
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
