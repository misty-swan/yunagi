// 各ページの本文を作る。Markdown の本文は site/content/<言語>/ と docs/ から取り込む（コピーを置かない）。
import { readFileSync } from "node:fs";
import { names, type Arch } from "../../scripts/release/package.ts";
import { join } from "node:path";
import { DOWNLOAD_BASE, RELEASES_URL, SITE, type Lang } from "./consts.ts";
import { esc } from "./html.ts";
import type { Latest } from "./latest.ts";
import { pagePath, renderPage, SCENE_SVG } from "./layout.ts";
import { renderMarkdown } from "./markdown.ts";
import { DESCRIPTIONS, HOME, PAGE_KEYS, UI, type PageKey } from "./text.ts";

export interface SiteInputs {
  /** リポジトリの直下 */
  root: string;
  /** 公開中（または今回含める）latest.json の読み取り結果。無ければ null（最初のリリースの前） */
  latest: Latest | null;
  /** 配布用の公開鍵（minisign の公開鍵ファイルの中身）。無ければ null（鍵の作成後に置く。07 の公式サイトの作り方） */
  releaseKey: string | null;
}

const read = (root: string, ...path: string[]): string => readFileSync(join(root, ...path), "utf8");

/** ページのファイルの置き場所（docs/ の元の文書。コピーしない）。 */
export function sourcePaths(lang: Lang): Partial<Record<PageKey, string[]>> {
  return {
    guide: ["docs", "user", lang, "guide.md"],
    news: ["site", "content", lang, "news.md"],
    download: ["site", "content", lang, "download.md"],
    verify: ["site", "content", lang, "verify.md"],
    requirements: ["site", "content", lang, "requirements.md"],
    support: ["site", "content", lang, "support.md"],
    security: ["site", "content", lang, "security.md"],
  };
}

/** 変更履歴の元（日本語は CHANGELOG.md、英語は CHANGELOG.en.md） */
function changelogPath(lang: Lang): string[] {
  return ["docs", "operations", lang === "ja" ? "CHANGELOG.md" : "CHANGELOG.en.md"];
}

function download(lang: Lang, latest: Latest | null): string {
  const ja = lang === "ja";
  if (latest === null) {
    return `<div class="notice"><p>${
      ja
        ? `最初のリリースの前です。ファイルの一覧は、最初のリリースを公開するとここに出ます。公開済みのファイルは <a href="${RELEASES_URL}">GitHub の Releases</a> にもあります。`
        : `This is before the first release. The list of files appears here once the first release is published. Published files are also on <a href="${RELEASES_URL}">GitHub Releases</a>.`
    }</p></div>`;
  }
  const v = latest.version;
  const base = `${DOWNLOAD_BASE}/v${v}`;
  const link = (name: string) => `<a href="${esc(`${base}/${name}`)}">${esc(name)}</a>`;
  // 出す組は latest.json の platforms にあるものだけ（公開しなかった組へのリンクを作らない）。
  // ファイル名は配布用 CI の定義（scripts/release/package.ts の names）を使う
  const all: Array<[string, string, Arch, string]> = [
    ["darwin-aarch64", "macOS", "arm64", names.macApp(v, "arm64")],
    ["darwin-x86_64", "macOS", "x86_64", names.macApp(v, "x86_64")],
    ["linux-x86_64", "Linux", "x86_64", names.appImage(v, "x86_64")],
    ["linux-aarch64", "Linux", "arm64", names.appImage(v, "arm64")],
    ["windows-x86_64", "Windows", "x86_64", names.setup(v, "x86_64")],
    ["windows-aarch64", "Windows", "arm64", names.setup(v, "arm64")],
  ];
  const rows = all.filter(([key]) => latest.platforms.includes(key));
  const head = ja ? ["OS", "アーキテクチャ", "ファイル", "ハッシュ"] : ["OS", "Architecture", "File", "Hash"];
  // 表を横にスクロールできる領域で包むのは、Markdown の変換（renderMarkdown）が、スロットの中の表も含めて行う
  const table = `<table>
<thead><tr>${head.map((h) => `<th scope="col">${esc(h)}</th>`).join("")}</tr></thead>
<tbody>
${rows.map(([, os, arch, name]) => `<tr><td>${os}</td><td>${arch}</td><td>${link(name)}</td><td>${link(`${name}.sha256`)}</td></tr>`).join("\n")}
</tbody>
</table>`;
  const lead = ja
    ? `最新版は <strong>${esc(v)}</strong> です。ファイルは GitHub の Releases（<a href="${esc(`${RELEASES_URL}/tag/v${v}`)}">v${esc(v)}</a>）にあります。`
    : `The latest version is <strong>${esc(v)}</strong>. The files are on GitHub Releases (<a href="${esc(`${RELEASES_URL}/tag/v${v}`)}">v${esc(v)}</a>).`;
  const sums = ja
    ? `全ファイルのハッシュをまとめた ${link("SHA256SUMS")} と、その署名 ${link("SHA256SUMS.sig")} もあります。確かめ方は<a href="${pagePath(lang, "verify")}">配布物の確かめ方</a>にあります。`
    : `${link("SHA256SUMS")}, which lists the hash of every file, and its signature ${link("SHA256SUMS.sig")} are also there. See <a href="${pagePath(lang, "verify")}">Verify a download</a> for how to check them.`;
  return `<p>${lead}</p>\n${table}\n<p>${sums}</p>`;
}

function verifyKey(lang: Lang, key: string | null): string {
  const ja = lang === "ja";
  if (key === null) {
    return `<div class="notice"><p>${
      ja
        ? "配布用の公開鍵は最初のリリースの前にここに掲載します。掲載するまでは、署名の確認はできません。"
        : "The public key for downloads will be published here before the first release. Until then, signatures cannot be checked."
    }</p></div>`;
  }
  const text = ja
    ? `公開鍵のファイルは <a href="/yunagi-release.pub">yunagi-release.pub</a> です。同じ内容をリポジトリの<a href="https://github.com/misty-swan/yunagi/blob/main/site/keys/release.pub"><code>site/keys/release.pub</code></a> でも見られます。2か所が同じであることも確かめてください。`
    : `The public key file is <a href="/yunagi-release.pub">yunagi-release.pub</a>. The same content is in the repository at <a href="https://github.com/misty-swan/yunagi/blob/main/site/keys/release.pub"><code>site/keys/release.pub</code></a>. Check that the two copies match.`;
  return `<p>${text}</p>\n<pre tabindex="0"><code>${esc(key.trimEnd())}</code></pre>`;
}

function installCards(lang: Lang, latest: Latest | null): string {
  const t = HOME[lang].download;
  const card = (label: string, cmd: string) =>
    `<div class="card"><h3>${esc(label)}</h3><pre tabindex="0"><code>${esc(cmd)}</code></pre></div>`;
  const status = latest
    ? `<p class="section-lead">${esc(t.latest(latest.version))}</p>`
    : `<div class="notice"><p>${esc(t.before)}</p></div>`;
  return `${status}
<div class="cards two">
${card(t.mac, `curl -fsSL ${SITE}/install.sh | sh`)}
${card(t.windows, `irm ${SITE}/install.ps1 | iex`)}
</div>`;
}

function home(lang: Lang, latest: Latest | null): string {
  const t = HOME[lang];
  // 日本語のページに出る英字のラベル（デザインガイド 7章）は、読み上げが英語になるよう lang="en" を付ける
  const en = lang === "ja" ? ' lang="en"' : "";
  const eyebrow = (text: string) => `<p class="eyebrow"${en}>${esc(text)}</p>`;
  const at = (p: string) => (p.startsWith("/") ? p : `${pagePath(lang, "home")}${p}`);
  const cards = t.features.cards
    .map(
      (c) =>
        `<div class="card">${eyebrow(c.label)}<h3>${esc(c.title)}</h3><p>${esc(c.text)}</p></div>`,
    )
    .join("\n");
  const chips = t.features.chips.map((c) => `<li>${esc(c)}</li>`).join("");
  const links = t.download.links.map((l) => `<li><a href="${esc(at(l.href))}">${esc(l.label)}</a></li>`).join("");
  const facts = t.security.facts
    .map((f) => {
      const text = f.href ? `<a href="${esc(at(f.href))}">${esc(f.title)}</a>` : esc(f.title);
      return `<li><strong>${text}</strong><span>${esc(f.text)}</span></li>`;
    })
    .join("\n");
  // 文字は景色（太陽・海・波）の上に載せない：コピーは上の空（.hero-sky）に置き、景色はその下の帯にする
  return `<section class="hero">
<div class="hero-sky">
<div class="wrap">
<div class="hero-copy">
${eyebrow(t.eyebrow)}
<h1>${esc(t.heading[0])}<br>${esc(t.heading[1])}</h1>
<p>${esc(t.lead)}</p>
<div class="actions">
<a class="button" href="${pagePath(lang, "download")}">${esc(t.primary)}</a>
<a href="${pagePath(lang, "guide")}">${esc(t.secondary)}</a>
</div>
<p class="hero-note">${esc(t.note)}</p>
</div>
</div>
</div>
${SCENE_SVG}
</section>
<section class="section" id="features">
<div class="wrap">
${eyebrow(t.features.eyebrow)}
<h2>${esc(t.features.heading)}</h2>
<div class="cards three">
${cards}
</div>
<ul class="chips">${chips}</ul>
</div>
</section>
<section class="section" id="install">
<div class="wrap">
${eyebrow(t.download.eyebrow)}
<h2>${esc(t.download.heading)}</h2>
<p class="section-lead">${esc(t.download.lead)}</p>
${installCards(lang, latest)}
<ul class="links">${links}</ul>
</div>
</section>
<section class="section" id="trust">
<div class="wrap split">
<div>
${eyebrow(t.security.eyebrow)}
<h2>${esc(t.security.heading)}</h2>
</div>
<ul class="facts">
${facts}
</ul>
</div>
</section>
<section class="section" id="guide">
<div class="wrap">
${eyebrow(t.guide.eyebrow)}
<h2>${esc(t.guide.heading)}</h2>
<p class="actions"><a class="button ghost" href="${pagePath(lang, "guide")}">${esc(t.guide.button)}</a></p>
</div>
</section>`;
}

/** 言語ごとの全ページ。キーは PageKey、値は完成した HTML。 */
export function buildPages(lang: Lang, inputs: SiteInputs): Record<PageKey, string> {
  const { root, latest, releaseKey } = inputs;
  const sources = sourcePaths(lang);
  const out = {} as Record<PageKey, string>;
  const tableLabel = UI[lang].tableLabel;

  for (const page of PAGE_KEYS) {
    const description = DESCRIPTIONS[lang][page];
    if (page === "home") {
      out[page] = renderPage({ lang, page, title: HOME[lang].title, description, body: home(lang, latest) });
      continue;
    }
    const src = read(root, ...(sources[page] as string[]));
    const slots: Record<string, string> = {};
    if (page === "download") slots.files = download(lang, latest);
    if (page === "verify") slots.key = verifyKey(lang, releaseKey);
    if (page === "news") {
      slots.changelog = renderMarkdown(read(root, ...changelogPath(lang)), { shift: 1, dropFirstH1: true, tableLabel }).html;
    }
    const { html, headings } = renderMarkdown(src, { slots, tableLabel });
    const h1 = headings.find((h) => h.depth === 1);
    if (!h1) throw new Error(`${lang}/${page}: 先頭に h1 が無い`);
    let body = html;
    if (page === "guide") {
      const items = headings
        .filter((h) => h.depth === 2)
        .map((h) => `<li><a href="#${esc(h.id)}">${esc(h.text)}</a></li>`)
        .join("\n");
      const toc = `<nav class="toc" aria-label="${esc(UI[lang].toc)}"><strong>${esc(UI[lang].toc)}</strong><ol>\n${items}\n</ol></nav>`;
      body = body.replace(/<\/h1>\n/, `</h1>\n${toc}\n`);
    }
    out[page] = renderPage({
      lang,
      page,
      title: h1.text,
      description,
      body: `<div class="wrap"><article class="doc">\n${body}\n</article></div>`,
    });
  }
  return out;
}
