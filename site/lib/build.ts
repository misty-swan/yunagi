// 公式サイトをビルドして出力先に書く。使うのは Node.js の標準の機能と marked だけ。
// 決まり：docs/release/07-build-and-distribution.md 2章「公式サイトの作り方」。
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, parse, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { LANGS, SITE } from "./consts.ts";
import { esc } from "./html.ts";
import { parseLatest, type Latest } from "./latest.ts";
import { buildPages } from "./pages.ts";
import { pagePath } from "./layout.ts";
import { PAGE_KEYS } from "./text.ts";

/** リポジトリの直下 */
export const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

/** minisign の公開鍵ファイル（2 行。1 行目はコメント、2 行目は `RW` で始まる 56 文字の base64） */
const MINISIGN_PUBKEY = /^untrusted comment: [^\n]+\nRW[A-Za-z0-9+/]{54}\n?$/;

export interface BuildOptions {
  root?: string;
  outDir: string;
  /** 取り込む latest.json のパス。無ければ含めない（最初のリリースの前） */
  latestJson?: string;
}

export interface BuildResult {
  latest: Latest | null;
  files: string[];
}

function write(outDir: string, rel: string, content: string | Uint8Array, files: string[]): void {
  const path = join(outDir, rel);
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
  files.push(rel);
}

function copy(root: string, from: string[], outDir: string, rel: string, files: string[]): void {
  const dest = join(outDir, rel);
  mkdirSync(dirname(dest), { recursive: true });
  cpSync(join(root, ...from), dest);
  files.push(rel);
}

/** ルートのページ（言語の振り分けはしない。日本語へ移す）と、見つからないときのページ */
function plain(lang: "ja" | "en", title: string, body: string, head = ""): string {
  return `<!doctype html>
<html lang="${lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)}</title>
<meta name="robots" content="noindex">
<meta name="color-scheme" content="light dark">
${head}<link rel="icon" href="/assets/favicon.svg" type="image/svg+xml">
<link rel="stylesheet" href="/assets/site.css">
</head>
<body>
<main>
<div class="wrap"><div class="doc">
${body}
</div></div>
</main>
</body>
</html>
`;
}

export function headersFile(): string {
  return `# Cloudflare Pages の _headers（docs/release/07-build-and-distribution.md 2章「公式サイトの作り方」）。
# 外部への読み込みは無いので、CSP は同じサイトのものだけを許す。
/*
  Content-Security-Policy: default-src 'none'; style-src 'self'; font-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'
  X-Content-Type-Options: nosniff
  X-Frame-Options: DENY
  Referrer-Policy: strict-origin-when-cross-origin
  Permissions-Policy: camera=(), microphone=(), geolocation=()

# 古い内容が使われ続けないよう、毎回確かめさせる（2章。公開後に curl -I で確かめる）
/latest.json
  Cache-Control: no-cache
  Content-Type: application/json; charset=utf-8

# 導入スクリプトはブラウザで読めるようにする（ダウンロードさせない）。読んでから実行する手順のため
/install.sh
  Cache-Control: no-cache
  Content-Type: text/plain; charset=utf-8

/install.ps1
  Cache-Control: no-cache
  Content-Type: text/plain; charset=utf-8

/yunagi-release.pub
  Cache-Control: no-cache
  Content-Type: text/plain; charset=utf-8
`;
}

export function redirectsFile(): string {
  // ルートはブラウザの言語で振り分けず、日本語へ移す（一時的な移動。決まりを変えうるので 302）
  return `/ /ja/ 302\n`;
}

/**
 * 出力先は、ビルドのたびに丸ごと消して作り直す。誤って別のフォルダ（リポジトリ自身・その上・ホーム）を
 * 指していたら消さずに止める。すでにあって空でないときは、前のビルドの出力（`_headers` と `ja/index.html` がある）のときだけ消す。
 */
function assertSafeOutDir(root: string, outDir: string): void {
  const out = resolve(outDir);
  const repo = resolve(root);
  const rel = relative(out, repo);
  // out がリポジトリ自身か、その上のフォルダ（リポジトリが out の中にある）
  if (rel === "" || !rel.startsWith("..")) throw new Error(`出力先 ${out} はリポジトリ自身かその上のフォルダなので消せない`);
  if (out === resolve(homedir()) || out === parse(out).root) throw new Error(`出力先 ${out} は消せない`);
  if (existsSync(out)) {
    if (!statSync(out).isDirectory()) throw new Error(`出力先 ${out} がフォルダではない`);
    const empty = readdirSync(out).length === 0;
    const previous = existsSync(join(out, "_headers")) && existsSync(join(out, "ja", "index.html"));
    if (!empty && !previous) throw new Error(`出力先 ${out} は空でなく、前のビルドの出力でもないので消さない`);
  }
}

export function build(options: BuildOptions): BuildResult {
  const root = options.root ?? ROOT;
  const { outDir } = options;
  const files: string[] = [];

  // 入力は、出力先を消す前に読む（出力先の中にある入力を消してしまわない）
  let latest: Latest | null = null;
  let latestText: string | null = null;
  if (options.latestJson !== undefined) {
    latestText = readFileSync(options.latestJson, "utf8");
    latest = parseLatest(latestText);
  }

  assertSafeOutDir(root, outDir);
  rmSync(outDir, { recursive: true, force: true });
  mkdirSync(outDir, { recursive: true });
  if (latestText !== null) write(outDir, "latest.json", latestText, files);

  const keyPath = join(root, "site", "keys", "release.pub");
  let releaseKey: string | null = null;
  if (existsSync(keyPath)) {
    releaseKey = readFileSync(keyPath, "utf8");
    if (!MINISIGN_PUBKEY.test(releaseKey)) {
      throw new Error("site/keys/release.pub が minisign の公開鍵の形（コメント行と RW で始まる 56 文字）ではない");
    }
    write(outDir, "yunagi-release.pub", releaseKey, files);
  }

  for (const lang of LANGS) {
    const pages = buildPages(lang, { root, latest, releaseKey });
    for (const page of PAGE_KEYS) {
      write(outDir, `${pagePath(lang, page).slice(1)}index.html`, pages[page], files);
    }
  }

  write(
    outDir,
    "index.html",
    plain(
      "ja",
      "Yunagi",
      `<h1>Yunagi</h1>
<p><a href="/ja/">日本語のページへ移動します。</a></p>
<p lang="en"><a href="/en/">English</a></p>`,
      `<meta http-equiv="refresh" content="0; url=/ja/">\n<link rel="canonical" href="${SITE}/ja/">\n`,
    ),
    files,
  );
  write(
    outDir,
    "404.html",
    plain(
      "ja",
      "404 — Yunagi",
      `<h1>404</h1>
<p>ページが見つかりません。<a href="/ja/">トップページ</a></p>
<p lang="en">Page not found. <a href="/en/">Home</a></p>`,
    ),
    files,
  );

  copy(root, ["site", "static", "site.css"], outDir, "assets/site.css", files);
  // アプリアイコンの元（src-tauri/icons/source/app-icon.svg）をそのまま使う。二重に持たない
  copy(root, ["src-tauri", "icons", "source", "app-icon.svg"], outDir, "assets/favicon.svg", files);
  // フォントは配布元のファイルを加工せずに置く（デザインガイド 4章）。許諾の文書も一緒に
  copy(root, ["src", "assets", "fonts", "inter", "InterVariable.woff2"], outDir, "assets/fonts/inter/InterVariable.woff2", files);
  copy(root, ["src", "assets", "fonts", "inter", "LICENSE.txt"], outDir, "assets/fonts/inter/LICENSE.txt", files);
  // 導入スクリプトの正本は install/。ビルドでコピーする（二重に持たない）
  copy(root, ["install", "install.sh"], outDir, "install.sh", files);
  copy(root, ["install", "install.ps1"], outDir, "install.ps1", files);

  write(outDir, "_headers", headersFile(), files);
  write(outDir, "_redirects", redirectsFile(), files);
  return { latest, files };
}
