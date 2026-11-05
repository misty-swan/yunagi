// 許可外ライセンスの検出と、THIRD_PARTY_LICENSES.md の生成（配布用 CI の検査ジョブ）。
// 決まりは docs/legal/third-party-licenses.md 2章と docs/release/07-build-and-distribution.md 3章。
//
//   node scripts/release/licenses.ts check               … 許可外のライセンスがあれば失敗する（終了コード 1）
//   node scripts/release/licenses.ts generate <出力先>   … THIRD_PARTY_LICENSES.md を作る
//
// リポジトリ直下で動かす。cargo-about（Rust の依存）と pnpm（JavaScript の依存）が要る。
// 許可するライセンスの一覧は license-policy.json。about.toml の accepted は、allowed と pending を合わせたもの。
import { escapeData } from "./audit.ts";
import { exec } from "./exec.ts";
import { readStringArray, replaceStringArray } from "./toml.ts";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

export type Policy = { allowed: string[]; pending: string[] };

/**
 * SPDX のライセンス式（`MIT OR Apache-2.0`・`A AND (B OR C)` など）を、許可する一覧で満たせるか。
 * OR はどれか 1 つ、AND はすべてが許可に入っているとき満たす。`WITH` 付きの式は、例外つきの別のライセンスとして
 * 扱い、一覧にそのままの形が無ければ満たさない。
 */
export function satisfies(expression: string, allowed: readonly string[]): boolean {
  const tokens = expression.replace(/([()])/g, " $1 ").split(/\s+/).filter(Boolean);
  let pos = 0;
  const parseOr = (): boolean => {
    let value = parseAnd();
    while (tokens[pos]?.toUpperCase() === "OR") {
      pos++;
      const rhs = parseAnd();
      value = value || rhs;
    }
    return value;
  };
  const parseAnd = (): boolean => {
    let value = parseTerm();
    while (tokens[pos]?.toUpperCase() === "AND") {
      pos++;
      const rhs = parseTerm();
      value = value && rhs;
    }
    return value;
  };
  const parseTerm = (): boolean => {
    const t = tokens[pos++];
    if (t === undefined) throw new Error(`ライセンス式が途中で終わっている: ${expression}`);
    if (t === "(") {
      const value = parseOr();
      if (tokens[pos++] !== ")") throw new Error(`ライセンス式の括弧が閉じていない: ${expression}`);
      return value;
    }
    let id = t.replace(/\+$/, "");
    if (tokens[pos]?.toUpperCase() === "WITH") {
      id = `${id} WITH ${tokens[pos + 1]}`;
      pos += 2;
    }
    return allowed.includes(id);
  };
  const result = parseOr();
  if (pos !== tokens.length) throw new Error(`ライセンス式を読み切れない: ${expression}`);
  return result;
}

/** about.toml の `accepted = [...]` の ID（コメントの行は読まない） */
export function readAccepted(toml: string): string[] {
  return readStringArray(toml, "accepted");
}

/** about.toml の accepted を、許可が決まったもの（allowed）だけにした設定を作る（許可外の検出用） */
export function strictAboutToml(toml: string, allowed: readonly string[]): string {
  return replaceStringArray(toml, "accepted", allowed);
}

/** 同梱するフォント（デザインガイド 4章）。ここに無いファイルは、同梱を認めない（third-party-licenses.md 2章の 5） */
export const FONTS = [
  { name: "Inter", spdx: "OFL-1.1", dir: "src/assets/fonts/inter", font: ["InterVariable.woff2"], licenses: ["LICENSE.txt"] },
  {
    name: "UDEV Gothic HS（JetBrains Mono と BIZ UDゴシックを含む）",
    spdx: "OFL-1.1",
    dir: "src/assets/fonts/udev-gothic",
    font: ["UDEVGothicHS-Regular.ttf"],
    licenses: ["LICENSE", "LICENSE_JetBrainsMono", "LICENSE_BIZUDGothic"],
  },
] as const;

const FONT_ROOT = "src/assets/fonts";

/** フォントのフォルダの食い違い（知らないフォルダ・ファイル、足りないライセンス文書）を返す。SHA-256 は src/styles/fonts.test.ts が確かめる */
export function checkFonts(root: string = FONT_ROOT): string[] {
  const problems: string[] = [];
  const dirs = readdirSync(root, { withFileTypes: true }).filter((d) => d.isDirectory()).map((d) => d.name);
  const known = FONTS.map((f) => f.dir.split("/").pop()!);
  for (const d of dirs) {
    if (!known.includes(d)) problems.push(`${root}/${d}: デザインガイド 4章に無いフォントのフォルダ（増やすときはデザインガイドに書いてから入れる）`);
  }
  for (const f of FONTS) {
    const dir = `${root}/${f.dir.split("/").pop()}`;
    let files: string[];
    try {
      files = readdirSync(dir);
    } catch {
      problems.push(`${dir}: フォルダが無い`);
      continue;
    }
    for (const need of [...f.font, ...f.licenses]) {
      if (!files.includes(need)) problems.push(`${dir}/${need}: 無い`);
    }
    for (const have of files) {
      if (![...f.font, ...f.licenses].includes(have as never)) problems.push(`${dir}/${have}: 知らないファイル`);
    }
  }
  return problems;
}

type CargoAboutJson = {
  licenses: { id: string; name: string; text: string; used_by: { crate: { name: string; version: string } }[] }[];
};

type JsPackage = { name: string; versions: string[]; paths: string[]; license: string };

function readPolicy(): Policy {
  return JSON.parse(readFileSync("scripts/release/license-policy.json", "utf8")) as Policy;
}

/** cargo-about を動かして JSON を得る。許可外があると、cargo-about が失敗して stderr に理由を出す */
function cargoAbout(config: string): { ok: true; json: CargoAboutJson } | { ok: false; error: string } {
  const dir = mkdtempSync(join(tmpdir(), "yunagi-about-"));
  try {
    const out = join(dir, "about.json");
    const r = exec("cargo", ["about", "generate", "--locked", "--format", "json", "-c", config, "-o", out]);
    if (r.status !== 0) return { ok: false, error: r.stderr.trim() };
    return { ok: true, json: JSON.parse(readFileSync(out, "utf8")) as CargoAboutJson };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/** `pnpm licenses list --prod` の配布物に入る JavaScript の依存。devDependencies は配布物に入らないので数えない */
function jsPackages(): JsPackage[] {
  const r = exec("pnpm", ["licenses", "list", "--prod", "--json"]);
  if (r.status !== 0) throw new Error(`pnpm licenses list が失敗した（終了コード ${r.status}）\n${r.stderr}`);
  const byLicense = JSON.parse(r.stdout) as Record<string, JsPackage[]>;
  return Object.values(byLicense).flat();
}

/** `LICENSE`・`LICENSE.md`・`LICENSE-MIT`・`LICENSE_APACHE-2.0`・`COPYING`・`NOTICE` など。ソースファイル（`license-utils.js`）は含めない */
export const LICENSE_FILE = /^(licen[cs]e|copying|notice)([-_.][A-Za-z0-9._-]*)?$/i;
const SOURCE_EXT = /\.(js|cjs|mjs|ts|json|spdx|d\.ts|map)$/i;

/** パッケージ（入っている版すべて）のライセンス文書。ディレクトリ（`LICENSES/` など）は読まない */
function licenseTexts(pkg: JsPackage): string[] {
  const texts = new Set<string>();
  for (const dir of pkg.paths) {
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      if (entry.isFile() && LICENSE_FILE.test(entry.name) && !SOURCE_EXT.test(entry.name)) {
        texts.add(readFileSync(join(dir, entry.name), "utf8").trim());
      }
    }
  }
  return [...texts];
}

function fence(text: string): string {
  const longest = Math.max(2, ...[...text.matchAll(/`+/g)].map((m) => m[0].length));
  return "`".repeat(longest + 1);
}

function block(text: string): string {
  const f = fence(text);
  return `${f}text\n${text.trim()}\n${f}\n`;
}

function renderDocument(rust: CargoAboutJson, js: JsPackage[]): string {
  const out: string[] = [];
  out.push("# サードパーティライセンス表記\n");
  out.push(
    "Yunagi は、次のオープンソースソフトウェアとフォントを使っています。この表記は、リリースごとに CI で生成しています（docs/legal/third-party-licenses.md）。\n",
  );

  out.push("## 同梱するフォント\n");
  for (const f of FONTS) {
    out.push(`### ${f.name}\n`);
    out.push(`ライセンス：${f.spdx}（SIL Open Font License, Version 1.1）\n`);
    for (const l of f.licenses) {
      out.push(`ライセンス文書 \`${l}\`\n`);
      out.push(block(readFileSync(`${f.dir}/${l}`, "utf8")));
    }
  }

  out.push("## JavaScript の依存（画面）\n");
  const jsGroups = new Map<string, { license: string; pkgs: string[]; text: string }>();
  for (const p of [...js].sort((a, b) => a.name.localeCompare(b.name))) {
    const text = licenseTexts(p).join("\n\n---\n\n");
    const key = `${p.license}\n${text}`;
    const g = jsGroups.get(key) ?? { license: p.license, pkgs: [], text };
    g.pkgs.push(`${p.name} ${p.versions.join(", ")}`);
    jsGroups.set(key, g);
  }
  for (const g of jsGroups.values()) {
    out.push(`### ${g.license}\n`);
    out.push(g.pkgs.map((p) => `- ${p}`).join("\n") + "\n");
    out.push(g.text ? block(g.text) : "（パッケージにライセンス文書のファイルが無い。パッケージの package.json の license 欄が上のとおり）\n");
  }

  out.push("## Rust の依存（アプリ本体・CLI）\n");
  for (const l of [...rust.licenses].sort((a, b) => a.id.localeCompare(b.id) || a.text.localeCompare(b.text))) {
    out.push(`### ${l.name}（${l.id}）\n`);
    const names = [...new Set(l.used_by.map((u) => `${u.crate.name} ${u.crate.version}`))].sort();
    out.push(names.map((n) => `- ${n}`).join("\n") + "\n");
    out.push(block(l.text));
  }
  return out.join("\n");
}

function main(argv: string[]): number {
  const policy = readPolicy();
  const aboutToml = readFileSync("about.toml", "utf8");
  const accepted = readAccepted(aboutToml);
  const expected = [...policy.allowed, ...policy.pending];
  if (JSON.stringify([...accepted].sort()) !== JSON.stringify([...expected].sort())) {
    console.error("about.toml の accepted が license-policy.json の allowed と pending を合わせたものと違う");
    return 1;
  }

  if (argv[0] === "check") {
    const problems: string[] = [];
    const dir = mkdtempSync(join(tmpdir(), "yunagi-about-"));
    try {
      const strict = join(dir, "about-strict.toml");
      writeFileSync(strict, strictAboutToml(aboutToml, policy.allowed));
      const rust = cargoAbout(strict);
      if (!rust.ok) problems.push(`Rust の依存に許可外のライセンスがある（cargo-about）:\n${rust.error}`);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
    for (const p of jsPackages()) {
      if (!satisfies(p.license, policy.allowed)) problems.push(`JavaScript の依存 ${p.name} のライセンス ${p.license} が許可外`);
    }
    problems.push(...checkFonts());
    if (problems.length > 0) {
      for (const p of problems) console.error(`::error title=許可外ライセンス::${escapeData(p.split("\n")[0])}`);
      // 外部の出力（cargo-about の stderr）が、行頭の `::` でワークフローコマンドとして読まれないよう字下げする
      console.error(problems.map((p) => p.split("\n").map((l) => `  ${l}`).join("\n")).join("\n\n"));
      console.error(
        `\n許可しているライセンス：${policy.allowed.join("、")}。許可するか決まっていないもの（YUN-96）：${policy.pending.join("、")}`,
      );
      return 1;
    }
    console.log("許可外のライセンスは無い（Rust・JavaScript の配布物に入る依存・同梱するフォント）");
    return 0;
  }

  if (argv[0] === "generate" && argv[1]) {
    const rust = cargoAbout("about.toml");
    if (!rust.ok) {
      console.error(`THIRD_PARTY_LICENSES.md を作れない（cargo-about）:\n${rust.error}`);
      return 1;
    }
    writeFileSync(argv[1], renderDocument(rust.json, jsPackages()));
    console.log(`${argv[1]} を作った`);
    return 0;
  }

  console.error("使い方: node scripts/release/licenses.ts check | generate <出力先>");
  return 64;
}

if (import.meta.main) process.exit(main(process.argv.slice(2)));
