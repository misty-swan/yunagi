// @vitest-environment node
// 公式サイトの検査（docs/release/07-build-and-distribution.md 2章「公式サイトの作り方」）。
// 実際にビルドして、出力（HTML・CSS・_headers など）を確かめる：リンク切れ、日英のページがそろうこと、
// 見出しの階層、外部の読み込みが無いこと、色のトークンがデザインガイドと同じで読めること、latest.json の扱い。
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { JSDOM } from "jsdom";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { build, headersFile, ROOT } from "./lib/build.ts";
import { ISSUES_URL, LANGS, SITE, SUPPORT_MAIL } from "./lib/consts.ts";
import { fetchPublishedLatest, parseLatest } from "./lib/latest.ts";
import { renderMarkdown, slugify } from "./lib/markdown.ts";
import { PAGE_KEYS } from "./lib/text.ts";

const read = (path: string): string => readFileSync(join(ROOT, path), "utf8");

const LATEST = `{
  "version": "1.2.3",
  "notes": "test",
  "pub_date": "2026-12-01T00:00:00Z",
  "platforms": {
    "darwin-aarch64": { "url": "https://github.com/misty-swan/yunagi/releases/download/v1.2.3/Yunagi_1.2.3_arm64.app.tar.gz", "signature": "c2ln" },
    "linux-x86_64": { "url": "https://github.com/misty-swan/yunagi/releases/download/v1.2.3/Yunagi_1.2.3_x86_64.AppImage", "signature": "c2ln" }
  }
}
`;

const tmp = mkdtempSync(join(tmpdir(), "yunagi-site-test-"));
const plainDir = join(tmp, "plain");
const withDir = join(tmp, "with-latest");
const latestPath = join(tmp, "latest.json");

beforeAll(() => {
  writeFileSync(latestPath, LATEST);
  build({ outDir: plainDir });
  build({ outDir: withDir, latestJson: latestPath });
});

afterAll(() => rmSync(tmp, { recursive: true, force: true }));

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)],
  );
}

const htmlFiles = (dir: string) => walk(dir).filter((f) => f.endsWith(".html"));
const dom = (file: string) => new JSDOM(readFileSync(file, "utf8")).window.document;

describe("出力の構成", () => {
  it("日本語と英語のページが同じ組でそろう（トップ・各ページ）", () => {
    const list = (lang: string) =>
      walk(join(plainDir, lang)).map((f) => relative(join(plainDir, lang), f)).sort();
    expect(list("ja")).toEqual(list("en"));
    expect(list("ja").length).toBe(PAGE_KEYS.length);
    for (const lang of LANGS) {
      for (const f of list(lang)) expect(f.endsWith("index.html"), f).toBe(true);
    }
  });

  it("必要な公開物がある（導入スクリプトはリポジトリの install/ と同じ。latest.json は渡さなければ無い）", () => {
    for (const f of ["index.html", "404.html", "_headers", "_redirects", "assets/site.css", "assets/favicon.svg"]) {
      expect(existsSync(join(plainDir, f)), f).toBe(true);
    }
    expect(readFileSync(join(plainDir, "install.sh"), "utf8")).toBe(read("install/install.sh"));
    expect(readFileSync(join(plainDir, "install.ps1"), "utf8")).toBe(read("install/install.ps1"));
    expect(existsSync(join(plainDir, "latest.json"))).toBe(false);
    expect(readFileSync(join(withDir, "latest.json"), "utf8")).toBe(LATEST);
  });

  it("フォントは配布元のファイルのまま（加工しない）で、許諾の文書が一緒にある", () => {
    for (const f of ["InterVariable.woff2", "LICENSE.txt"]) {
      expect(readFileSync(join(plainDir, "assets/fonts/inter", f))).toEqual(
        readFileSync(join(ROOT, "src/assets/fonts/inter", f)),
      );
    }
  });

  it("ルートは言語で振り分けず日本語へ移す", () => {
    expect(readFileSync(join(plainDir, "_redirects"), "utf8")).toContain("/ /ja/ 302");
    const root = readFileSync(join(plainDir, "index.html"), "utf8");
    expect(root).toContain('http-equiv="refresh" content="0; url=/ja/"');
    expect(root).toContain('href="/en/"');
  });

  it("_headers：/latest.json は no-cache、導入スクリプトはブラウザで読める、CSP は同じサイトだけ", () => {
    const h = headersFile();
    expect(h).toMatch(/^\/latest\.json\n(?: {2}.+\n)*? {2}Cache-Control: no-cache$/m);
    expect(h).toMatch(/^\/install\.sh\n(?: {2}.+\n)*? {2}Content-Type: text\/plain; charset=utf-8$/m);
    expect(h).toMatch(/^\/install\.ps1\n(?: {2}.+\n)*? {2}Content-Type: text\/plain; charset=utf-8$/m);
    expect(h).toContain("default-src 'none'; style-src 'self'; font-src 'self'; img-src 'self'");
    expect(readFileSync(join(plainDir, "_headers"), "utf8")).toBe(h);
  });
});

describe("各ページの HTML", () => {
  const dirs = [plainDir, withDir];
  const allFiles = () => dirs.flatMap(htmlFiles);

  it("lang・h1・見出しの階層・id の重複", () => {
    for (const file of allFiles()) {
      const rel = relative(tmp, file);
      const doc = dom(file);
      const lang = doc.documentElement.getAttribute("lang");
      const m = /\/(ja|en)\//.exec(`/${relative(tmp, file).split("/").slice(1).join("/")}`);
      if (m) expect(lang, rel).toBe(m[1]);
      else expect(lang, rel).toBe("ja");
      expect(doc.title.length, rel).toBeGreaterThan(0);
      expect(doc.querySelectorAll("h1").length, rel).toBe(1);
      let prev = 0;
      for (const h of doc.querySelectorAll("h1,h2,h3,h4,h5,h6")) {
        const level = Number(h.tagName[1]);
        expect(level - prev, `${rel}: ${h.textContent} の見出しが飛んでいる`).toBeLessThanOrEqual(1);
        prev = level;
      }
      const ids = [...doc.querySelectorAll("[id]")].map((e) => e.id);
      expect(new Set(ids).size, `${rel}: id の重複`).toBe(ids.length);
      expect(doc.querySelector("main"), rel).not.toBeNull();
      // ルートの移動ページと 404 は、ヘッダーの無い簡単なページ
      if (m) {
        expect(doc.querySelector("main#main"), rel).not.toBeNull();
        expect(doc.querySelector('a.skip[href="#main"]'), rel).not.toBeNull();
      }
    }
  });

  it("外部の読み込み・スクリプト・インラインの style・イベント属性が無い", () => {
    for (const file of allFiles()) {
      const rel = relative(tmp, file);
      const doc = dom(file);
      expect(doc.querySelectorAll("script,iframe,img,object,embed,form,video,audio,source,style,input,button"), rel).toHaveLength(0);
      for (const el of doc.querySelectorAll("*")) {
        expect(el.hasAttribute("style"), `${rel}: <${el.tagName}> に style 属性`).toBe(false);
        for (const a of el.getAttributeNames()) expect(a.startsWith("on"), `${rel}: ${a}`).toBe(false);
      }
      for (const l of doc.querySelectorAll("link[href]")) {
        const rel2 = l.getAttribute("rel");
        const href = l.getAttribute("href") ?? "";
        if (rel2 === "stylesheet" || rel2 === "icon") expect(href.startsWith("/assets/"), `${rel}: ${href}`).toBe(true);
        else expect(href.startsWith(SITE), `${rel}: ${href}`).toBe(true);
      }
    }
  });

  it("リンク切れが無い（内部のリンクは出力の中のファイルと見出しの id に着く）", () => {
    // 見出しの id は、行き先のページごとに 1 回だけ読む（リンクのたびに読み直すと、全体を並列に流したとき 5 秒を超えた）
    const idsOf = new Map<string, Set<string>>();
    const ids = (file: string) => {
      let found = idsOf.get(file);
      if (!found) idsOf.set(file, (found = new Set([...dom(file).querySelectorAll("[id]")].map((e) => e.id))));
      return found;
    };
    for (const dir of dirs) {
      for (const file of htmlFiles(dir)) {
        const rel = relative(tmp, file);
        const doc = dom(file);
        for (const a of doc.querySelectorAll("a[href]")) {
          const href = a.getAttribute("href") ?? "";
          if (/^(https:\/\/|mailto:)/.test(href)) continue;
          expect(href, `${rel}: 相対リンク`).toMatch(/^(\/|#)/);
          const [pathPart, frag] = href.split("#");
          let target = file;
          if (pathPart !== "") {
            const p = join(dir, decodeURIComponent(pathPart));
            target = pathPart.endsWith("/") ? join(p, "index.html") : p;
            expect(existsSync(target) && statSync(target).isFile(), `${rel}: ${href} の行き先が無い`).toBe(true);
          }
          if (frag) {
            expect(target.endsWith(".html"), `${rel}: ${href}`).toBe(true);
            expect(ids(target).has(decodeURIComponent(frag)), `${rel}: ${href} の見出しが無い`).toBe(true);
          }
        }
      }
    }
    // 全ページの HTML を読むので、機械が忙しいと標準の 5 秒を超える（変更前の main でも超えた）。
  }, 30_000);

  it("日英で同じ構成（見出しの並び・コードの囲み・表・リンクの数）", () => {
    for (const dir of dirs) {
      for (const page of PAGE_KEYS) {
        const slug = page === "home" ? "" : `${page}/`;
        const shape = (lang: string) => {
          const doc = dom(join(dir, lang, slug, "index.html"));
          return {
            headings: [...doc.querySelectorAll("main h1,main h2,main h3,main h4")].map((h) => h.tagName),
            pre: doc.querySelectorAll("main pre").length,
            tables: doc.querySelectorAll("main table").length,
            links: doc.querySelectorAll("main a").length,
          };
        };
        expect(shape("en"), page).toEqual(shape("ja"));
      }
    }
  });

  it("言語の切り替えは同じページの別の言語に着き、hreflang の対がある", () => {
    for (const page of PAGE_KEYS) {
      const slug = page === "home" ? "" : `${page}/`;
      const ja = dom(join(plainDir, "ja", slug, "index.html"));
      const en = dom(join(plainDir, "en", slug, "index.html"));
      expect(ja.querySelector('.lang a[hreflang="en"]')?.getAttribute("href")).toBe(`/en/${slug}`);
      expect(en.querySelector('.lang a[hreflang="ja"]')?.getAttribute("href")).toBe(`/ja/${slug}`);
      expect(ja.querySelector('link[rel="canonical"]')?.getAttribute("href")).toBe(`${SITE}/ja/${slug}`);
      expect(en.querySelector('link[hreflang="ja"]')?.getAttribute("href")).toBe(`${SITE}/ja/${slug}`);
    }
  });

  it("表と等幅の囲みはキーボードで横に動かせる", () => {
    const doc = dom(join(plainDir, "ja", "guide", "index.html"));
    expect(doc.querySelectorAll("main table").length).toBeGreaterThan(0);
    for (const t of doc.querySelectorAll("main table")) {
      const wrap = t.parentElement as HTMLElement;
      expect(wrap.getAttribute("tabindex")).toBe("0");
      expect(wrap.getAttribute("role")).toBe("region");
      expect(wrap.getAttribute("aria-label")).toBeTruthy();
    }
    for (const pre of doc.querySelectorAll("main pre")) expect(pre.getAttribute("tabindex")).toBe("0");
  });
});

describe("取り込む文書", () => {
  it("利用者ガイドは docs/user/ の見出しを全部取り込み、目次が付く（コピーを置かない）", () => {
    for (const lang of LANGS) {
      const src = read(`docs/user/${lang}/guide.md`);
      const doc = dom(join(plainDir, lang, "guide", "index.html"));
      const h2src = [...src.matchAll(/^## (.+)$/gm)].map((m) => m[1]);
      const h2out = [...doc.querySelectorAll("article h2")].map((h) => h.textContent);
      expect(h2out).toEqual(h2src);
      expect(doc.querySelectorAll("nav.toc li").length).toBe(h2src.length);
      expect(existsSync(join(ROOT, "site", "content", lang, "guide.md"))).toBe(false);
    }
  });

  it("変更履歴は docs/operations/ の CHANGELOG（日本語）と CHANGELOG.en.md（英語）から取り込み、記載ルールのコメントは出さない", () => {
    for (const [lang, file] of [["ja", "CHANGELOG.md"], ["en", "CHANGELOG.en.md"]] as const) {
      const src = read(`docs/operations/${file}`);
      const html = readFileSync(join(plainDir, lang, "news", "index.html"), "utf8");
      const items = [...src.matchAll(/^- (.+)$/gm)].map((m) => m[1]).filter((t) => !t.startsWith("区分") && !t.includes("Sections"));
      expect(html).toContain("Unreleased");
      expect(html).not.toContain("記載ルール");
      expect(html).not.toContain("&lt;!--");
      expect(html).not.toContain("vX.Y.Z");
      expect(items.length).toBeGreaterThan(5);
    }
    // 日英の変更履歴は、区分と項目の数がそろう
    const count = (f: string) => ({
      h3: (read(f).match(/^### /gm) ?? []).length,
      items: (read(f).split("<!--")[0].match(/^- /gm) ?? []).length,
    });
    expect(count("docs/operations/CHANGELOG.en.md")).toEqual(count("docs/operations/CHANGELOG.md"));
  });

  it("Markdown の変換：生の HTML・画像・サイト外へ出ない相対リンクは止める。スロットは置き換える", () => {
    const opts = { tableLabel: "t" };
    expect(renderMarkdown("<b>x</b>", opts).html).toContain("&lt;b&gt;");
    expect(() => renderMarkdown("![a](/x.png)", opts)).toThrow(/画像/);
    expect(() => renderMarkdown("[a](../x.md)", opts)).toThrow(/リンク/);
    expect(() => renderMarkdown("[a](http://example.com)", opts)).toThrow(/リンク/);
    expect(renderMarkdown("a\n<!-- slot:k -->\nb", { ...opts, slots: { k: "<i>K</i>" } }).html).toContain("<i>K</i>");
    expect(() => renderMarkdown("<!-- slot:k -->", opts)).toThrow(/スロット/);
    const r = renderMarkdown("# A\n\n## 8. アンインストール\n\n## B\n\n## B\n", opts);
    expect(r.headings.map((h) => h.id)).toEqual(["a", "8-アンインストール", "b", "b-2"]);
    expect(slugify("macOS・Linux")).toBe("macoslinux");
  });

  it("HTML コメントは取り除くが、コードの囲み（``` と ~~~）と行内コードの中は残す。閉じていないコメントは止める", () => {
    const opts = { tableLabel: "t" };
    expect(renderMarkdown("a <!-- x --> b\n\n<!--\nrule\n-->\n\nc", opts).html).not.toContain("rule");
    const kept = renderMarkdown(
      "~~~\n<!-- keep -->\n~~~\n\n```\n<!-- keep2 -->\n```\n\n    <!-- indented -->\n\n````md\n```sh\n<!-- nested -->\n```\n````\n\n`<!-- slot:x -->` と書く",
      opts,
    ).html;
    for (const s of ["keep", "keep2", "indented", "nested"]) expect(kept, s).toContain(`&lt;!-- ${s} --&gt;`);
    expect(kept).toContain("<code>&lt;!-- slot:x --&gt;</code>");
    expect(() => renderMarkdown("a\n<!-- 閉じ忘れ\nb", opts)).toThrow(/閉じていない/);
  });

  it("生の HTML のブロックは段落にして、書いたとおりの文字で出す。見出しの文字は飾りを除いて拾う", () => {
    const opts = { tableLabel: "t" };
    expect(renderMarkdown("<div>raw</div>\n", opts).html).toContain("<p>&lt;div&gt;raw&lt;/div&gt;</p>");
    const r = renderMarkdown("## **`x` オプション** と [リンク](/a/)\n", opts);
    expect(r.headings[0].text).toBe("x オプション と リンク");
    expect(r.headings[0].id).toBe("x-オプション-と-リンク");
  });
});

describe("latest.json", () => {
  it("形の検査：版の形・version の位置・platforms・https", () => {
    expect(parseLatest(LATEST)).toEqual({
      version: "1.2.3",
      pubDate: "2026-12-01T00:00:00Z",
      platforms: ["darwin-aarch64", "linux-x86_64"],
    });
    expect(() => parseLatest("{")).toThrow(/JSON/);
    expect(() => parseLatest('{"version":"v1.2.3","platforms":{}}')).toThrow(/version/);
    expect(() => parseLatest('{"platforms":{"linux-x86_64":{"url":"https://x","signature":"s"}},"version":"1.2.3"}')).toThrow(/より後ろ/);
    expect(() => parseLatest('{"version":"1.2.3","platforms":{}}')).toThrow(/空/);
    expect(() => parseLatest('{"version":"1.2.3","platforms":{"freebsd":{"url":"https://x","signature":"s"}}}')).toThrow(/未知/);
    expect(() => parseLatest('{"version":"1.2.3","platforms":{"linux-x86_64":{"url":"http://x","signature":"s"}}}')).toThrow(/https/);
    expect(() => parseLatest('{"version":"1.2.3","platforms":{"linux-x86_64":{"url":"https://x"}}}')).toThrow(/signature/);
  });

  it("公開中の取得：200 は取り込む、404 は『まだ公開していない』、それ以外の失敗は『取れなかった』", async () => {
    const res = (status: number, body = "") => ({
      fetchImpl: (async () => new Response(body, { status })) as typeof fetch,
      delayMs: 0,
    });
    const url = "https://x/latest.json";
    expect(await fetchPublishedLatest(url, res(200, LATEST))).toMatchObject({ kind: "ok" });
    expect(await fetchPublishedLatest(url, res(404))).toMatchObject({ kind: "missing" });
    expect(await fetchPublishedLatest(url, res(503))).toMatchObject({ kind: "error" });
    expect(await fetchPublishedLatest(url, res(403))).toMatchObject({ kind: "error" });
    expect(await fetchPublishedLatest(url, res(200, "{}"))).toMatchObject({ kind: "error" });
    const boom = (async () => {
      throw new Error("ECONNREFUSED");
    }) as typeof fetch;
    expect(await fetchPublishedLatest(url, { fetchImpl: boom, delayMs: 0 })).toMatchObject({ kind: "error" });
  });

  it("一時的な失敗（5xx・通信の失敗）はやり直す。404・形の誤りはやり直さない。時間切れの信号を渡す", async () => {
    const calls: number[] = [];
    const flaky = (async (_url: unknown, init?: RequestInit) => {
      expect(init?.signal).toBeInstanceOf(AbortSignal);
      calls.push(1);
      if (calls.length === 1) return new Response("", { status: 502 });
      if (calls.length === 2) throw new Error("ECONNRESET");
      return new Response(LATEST, { status: 200 });
    }) as typeof fetch;
    expect(await fetchPublishedLatest("https://x/latest.json", { fetchImpl: flaky, delayMs: 0 })).toMatchObject({ kind: "ok" });
    expect(calls).toHaveLength(3);
    let n = 0;
    const missing = (async () => (n++, new Response("", { status: 404 }))) as typeof fetch;
    await fetchPublishedLatest("https://x/latest.json", { fetchImpl: missing, delayMs: 0 });
    expect(n).toBe(1);
    // 本文を読む途中の失敗（接続が切れた）も、やり直す。429 も一時的な失敗として扱う
    let k = 0;
    const brokenBody = (async () => {
      k++;
      if (k === 1) {
        const body = new ReadableStream({ start: (c) => c.error(new Error("reset while reading")) });
        return new Response(body, { status: 200 });
      }
      if (k === 2) return new Response("", { status: 429 });
      return new Response(LATEST, { status: 200 });
    }) as typeof fetch;
    expect(await fetchPublishedLatest("https://x/latest.json", { fetchImpl: brokenBody, delayMs: 0 })).toMatchObject({ kind: "ok" });
    expect(k).toBe(3);
  });

  it("文の中に \\\"version\\\": があっても、本物の version の位置の検査は狂わない", () => {
    const text = '{"notes":"say \\"version\\": 9 and \\"platforms\\": x","version":"1.2.3","platforms":{"linux-x86_64":{"url":"https://x","signature":"s"}}}';
    expect(parseLatest(text).version).toBe("1.2.3");
  });

  it("--allow-missing が許すのは 404 だけ（5xx・形の誤りでは中止する）", () => {
    const src = read("site/fetch-latest.ts");
    expect(src).toContain('allowMissing && result.kind === "missing"');
  });

  it("含めたときは最新版を出し、含めないときは『最初のリリースの前』を出す", () => {
    for (const lang of LANGS) {
      const withHome = readFileSync(join(withDir, lang, "index.html"), "utf8");
      const plainHome = readFileSync(join(plainDir, lang, "index.html"), "utf8");
      expect(withHome).toContain("1.2.3");
      expect(plainHome).not.toContain("1.2.3");
      const withDl = readFileSync(join(withDir, lang, "download", "index.html"), "utf8");
      expect(withDl).toContain("https://github.com/misty-swan/yunagi/releases/download/v1.2.3/Yunagi_1.2.3_arm64.app.tar.gz");
      // 一覧は latest.json の platforms にある組だけ（この試験用の latest.json は macOS arm64 と Linux x86_64）
      expect(withDl).toContain("Yunagi_1.2.3_x86_64.AppImage.sha256");
      expect(withDl).not.toContain("Yunagi_1.2.3_x86_64-setup.exe");
      expect(withDl).not.toContain("Yunagi_1.2.3_x86_64.app.tar.gz");
      // 表を包む領域は 1 つだけ（スロットの中の表を二重に包まない）
      const doc = new JSDOM(withDl).window.document;
      for (const t of doc.querySelectorAll("main table")) {
        expect(t.closest(".scroll")?.parentElement?.closest(".scroll")).toBeNull();
      }
      expect(readFileSync(join(plainDir, lang, "download", "index.html"), "utf8")).toContain("github.com/misty-swan/yunagi/releases");
    }
  });

  it("壊れた latest.json ではビルドを止める", () => {
    const bad = join(tmp, "bad.json");
    writeFileSync(bad, '{"version":"x"}');
    expect(() => build({ outDir: join(tmp, "bad-out"), latestJson: bad })).toThrow(/version/);
  });

  it("出力先を消す前に入力を読む。リポジトリやその上、関係ないフォルダは消さずに止める", () => {
    // 入力が出力先の中にあっても、先に読むので消えて失敗しない
    const out = join(tmp, "inside");
    build({ outDir: out });
    writeFileSync(join(out, "in.json"), LATEST);
    expect(() => build({ outDir: out, latestJson: join(out, "in.json") })).not.toThrow();
    expect(readFileSync(join(out, "latest.json"), "utf8")).toBe(LATEST);
    // リポジトリ自身・その上
    expect(() => build({ outDir: ROOT })).toThrow(/消せない/);
    expect(() => build({ outDir: join(ROOT, "..") })).toThrow(/消せない/);
    // 空でなく、前のビルドの出力でもない
    const other = join(tmp, "other");
    mkdirSync(other);
    writeFileSync(join(other, "keep.txt"), "x");
    expect(() => build({ outDir: other })).toThrow(/消さない/);
    expect(existsSync(join(other, "keep.txt"))).toBe(true);
  });
});

describe("配布用の公開鍵", () => {
  /** 試験用に、ビルドに要るものだけを写した作業用のルート */
  function fakeRoot(): string {
    const root = join(tmp, `root-${Math.random().toString(36).slice(2)}`);
    for (const p of ["site/content", "site/static", "docs/user", "docs/operations", "install", "src/assets/fonts/inter", "src-tauri/icons/source"]) {
      mkdirSync(join(root, p), { recursive: true });
      cpSync(join(ROOT, p), join(root, p), { recursive: true });
    }
    return root;
  }
  const KEY = `untrusted comment: minisign public key: 0123456789ABCDEF\nRWQ${"A".repeat(53)}\n`;

  it("鍵が無い間は『最初のリリースの前に掲載』の案内を出し、鍵のファイルは出さない", () => {
    const root = fakeRoot(); // site/keys が無い作業用のルート
    expect(existsSync(join(root, "site", "keys"))).toBe(false);
    const out = join(tmp, "nokey-out");
    build({ root, outDir: out });
    expect(existsSync(join(out, "yunagi-release.pub"))).toBe(false);
    expect(readFileSync(join(out, "ja", "verify", "index.html"), "utf8")).toContain("最初のリリースの前に、ここに掲載します");
    expect(readFileSync(join(out, "en", "verify", "index.html"), "utf8")).toContain("will be published here before the first release");
  });

  it("リポジトリに置いた site/keys/release.pub を、ビルドが /yunagi-release.pub として公開し、日英のページに値が出る", () => {
    const key = readFileSync(join(ROOT, "site", "keys", "release.pub"), "utf8");
    // 1 行目の鍵 ID（16 桁の 16 進）が、2 行目の中身（"Ed" + 鍵 ID 8 バイト + 公開鍵 32 バイト）の鍵 ID と同じこと
    expect(key).toMatch(/^untrusted comment: minisign public key:? [0-9A-F]{16}\nRW[A-Za-z0-9+/]{54}\n$/);
    const raw = Buffer.from(key.split("\n")[1], "base64");
    expect(raw.length).toBe(42);
    expect(raw.subarray(0, 2).toString("latin1")).toBe("Ed");
    const idFromBody = [...raw.subarray(2, 10)].reverse().map((b) => b.toString(16).padStart(2, "0")).join("").toUpperCase();
    expect(key.split("\n")[0].endsWith(idFromBody)).toBe(true);
    expect(readFileSync(join(plainDir, "yunagi-release.pub"), "utf8")).toBe(key);
    const [comment, body] = key.trimEnd().split("\n");
    for (const lang of ["ja", "en"]) {
      const html = readFileSync(join(plainDir, lang, "verify", "index.html"), "utf8");
      expect(html).toContain(comment);
      expect(html).toContain(body);
      expect(html).not.toContain("ここに掲載します");
      expect(html).not.toContain("will be published here");
    }
  });

  it("site/keys/release.pub を置くと、鍵のファイルと本文に出る。形が違えばビルドを止める", () => {
    const root = fakeRoot();
    mkdirSync(join(root, "site", "keys"));
    writeFileSync(join(root, "site", "keys", "release.pub"), KEY);
    const out = join(tmp, "key-out");
    build({ root, outDir: out });
    expect(readFileSync(join(out, "yunagi-release.pub"), "utf8")).toBe(KEY);
    expect(readFileSync(join(out, "en", "verify", "index.html"), "utf8")).toContain(`RWQ${"A".repeat(53)}`);
    writeFileSync(join(root, "site", "keys", "release.pub"), "bogus\n");
    expect(() => build({ root, outDir: join(tmp, "key-bad") })).toThrow(/公開鍵の形/);
  });
});

describe("色と文字（デザインガイド 3章・4章）", () => {
  const css = read("site/static/site.css");
  const guide = read("docs/design/06-design-guide.md");

  /** `--名前: 値;` を取り出す（空白を除き、小文字にし、`0.5` を `.5` にそろえる） */
  const tokens = (block: string): Map<string, string> => {
    const norm = (v: string) => v.replace(/\s+/g, "").toLowerCase().replace(/(^|[(,])0\./g, "$1.");
    return new Map([...block.matchAll(/--([a-z0-9-]+):\s*([^;]+);/g)].map((m) => [m[1], norm(m[2])]));
  };
  const lightBlock = css.slice(css.indexOf(":root {"), css.indexOf("}", css.indexOf(":root {")));
  // ダークは画面だけに当てる（印刷で明るい文字が紙で見えなくなるのを防ぐ）
  const darkStart = css.indexOf("@media screen and (prefers-color-scheme: dark)");
  if (darkStart < 0) throw new Error("ダークのトークンの規則が見つからない");
  const darkBlock = css.slice(css.indexOf(":root {", darkStart), css.indexOf("}", css.indexOf(":root {", darkStart)));
  const light = tokens(lightBlock);
  const dark = tokens(darkBlock);

  /** 表の行（`| \`--名前\` | ライト | ダーク | …`）から、トークンの値を読む */
  const readTable = (table: string) => {
    const out = { light: new Map<string, string>(), dark: new Map<string, string>() };
    for (const r of table.matchAll(/^\| `--([a-z0-9-]+)`(?: \/ `--([a-z0-9-]+)`)? \| ([^|]+) \| ([^|]+) \|/gm)) {
      if (!r[3].includes("#") && !r[3].includes("rgba")) continue;
      const split = (v: string) => v.split(" / ").map((x) => tokens(`--x: ${x};`).get("x"));
      const [l, d] = [split(r[3]), split(r[4])];
      [r[1], r[2]].filter(Boolean).forEach((n, i) => {
        out.light.set(n, l[i] ?? "");
        out.dark.set(n, d[i] ?? "");
      });
    }
    return out;
  };
  const table = guide.slice(guide.indexOf("## 3. 色"), guide.indexOf("## 4. 文字"));
  const siteTable = guide.slice(guide.indexOf("## 10. 公式サイト"), guide.indexOf("## 11. フォーム部品"));
  const appTokens = readTable(table);
  const siteTokens = readTable(siteTable);
  // アプリの凪メーターの空（--sky-1・--sky-2）は、サイトでは使わない（サイトの空は --site-sky-1〜3。06 10章）ので、CSS に置かない
  const APP_ONLY = ["sky-1", "sky-2"];
  const withoutAppOnly = (m: Map<string, string>) => new Map([...m].filter(([k]) => !APP_ONLY.includes(k)));
  const expected = {
    light: new Map([...withoutAppOnly(appTokens.light), ...siteTokens.light]),
    dark: new Map([...withoutAppOnly(appTokens.dark), ...siteTokens.dark]),
  };

  it("ライト・ダークの値が、デザインガイド 3章の表（アプリ専用の空を除く）と 10章のサイト専用トークンの表と同じ（表にない色を足していない）", () => {
    expect(appTokens.light.size).toBe(26);
    for (const k of APP_ONLY) expect(light.has(k) || dark.has(k), `--${k} はサイトに置かない`).toBe(false);
    expect([...siteTokens.light.keys()]).toEqual(["site-sky-1", "site-sky-2", "site-sky-3"]);
    // 3章と同じ名前を 10章で上書きしていない
    for (const k of siteTokens.light.keys()) expect(appTokens.light.has(k), k).toBe(false);
    const own = (m: Map<string, string>) => new Map([...m].filter(([k]) => k !== "on-accent"));
    expect(own(light)).toEqual(expected.light);
    expect(own(dark)).toEqual(expected.dark);
    expect(light.get("on-accent")).toBe("var(--surface)");
    expect(dark.get("on-accent")).toBe("var(--bg)");
  });

  it("色の値は、トークンの定義の外に書かない。文字の色は決まったトークンだけ", () => {
    const outside = css
      .split("\n")
      .filter((l) => !/^\s*--[a-z0-9-]+:/.test(l))
      .join("\n");
    expect(outside).not.toMatch(/#[0-9a-f]{3,8}\b|rgba?\(|hsla?\(/i);
    const defined = new Set([...light.keys()]);
    for (const m of css.matchAll(/var\(--([a-z0-9-]+)\)/g)) expect(defined.has(m[1]), `--${m[1]}`).toBe(true);
    for (const m of css.matchAll(/(?<![-\w])color:\s*([^;]+);/g)) {
      expect(m[1].trim(), "color").toMatch(/^(var\(--(text|text-muted|accent-strong|on-accent)\)|inherit)$/);
    }
  });

  /** sRGB の相対輝度から求めるコントラスト比（WCAG） */
  const lum = (hex: string): number => {
    const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
    const f = (v: number) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
    return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]);
  };
  const ratio = (a: string, b: string): number => {
    const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
    return (x + 0.05) / (y + 0.05);
  };

  it("文字のコントラスト：本文 7:1、補助・強調 4.5:1 以上（背景・サイド・面と、文字を載せるヒーローの空の 2 段に対して）", () => {
    for (const [name, t] of [["ライト", light], ["ダーク", dark]] as const) {
      // 文字を載せる空は --site-sky-1・--site-sky-2。--site-sky-3 は水平線の近くで、文字を載せない（下の構造の試験）
      const bgs = ["bg", "side", "surface", "site-sky-1", "site-sky-2"].map((k) => t.get(k) as string);
      for (const bg of bgs) {
        expect(ratio(t.get("text") as string, bg), `${name} text/${bg}`).toBeGreaterThanOrEqual(7);
        expect(ratio(t.get("text-muted") as string, bg), `${name} muted/${bg}`).toBeGreaterThanOrEqual(4.5);
        expect(ratio(t.get("accent-strong") as string, bg), `${name} accent-strong/${bg}`).toBeGreaterThanOrEqual(4.5);
      }
    }
    // 主なボタン：--accent-strong の面に --on-accent の文字（ライトは --surface、ダークは --bg）
    expect(ratio(light.get("surface") as string, light.get("accent-strong") as string)).toBeGreaterThanOrEqual(4.5);
    expect(ratio(dark.get("bg") as string, dark.get("accent-strong") as string)).toBeGreaterThanOrEqual(4.5);
    // 枠のある操作部品（ghost ボタンの枠 --ring）は 3:1
    for (const t of [light, dark]) {
      for (const k of ["bg", "surface"]) {
        expect(ratio(t.get("ring") as string, t.get(k) as string), `ring/${k}`).toBeGreaterThanOrEqual(3);
      }
    }
  });

  it("動きは波の線だけ（transform だけ・動きを減らす設定で止まる）で、トランジションが無く、外部の読み込みが無い", () => {
    const bare = css.replace(/\/\*[\s\S]*?\*\//g, "");
    expect(bare).not.toMatch(/\btransition\b/);
    // @keyframes は 1 つで、中身は transform だけ（レイアウトを動かさない）
    const frames = [...bare.matchAll(/@keyframes\s+([\w-]+)\s*\{/g)];
    expect(frames.map((m) => m[1])).toEqual(["wave-drift"]);
    // 最上位の規則は、行頭の `}` で終わる（この CSS の書き方）。終わりが見つからない・空のときは、すり抜けずに失敗させる
    const topLevelBlock = (from: number): string => {
      const end = bare.indexOf("\n}\n", from);
      expect(end, "最上位の規則の終わりが見つからない").toBeGreaterThan(from);
      return bare.slice(from, end);
    };
    const framesBody = topLevelBlock(frames[0].index);
    expect([...framesBody.matchAll(/^\s+([a-z-]+):/gm)].map((m) => m[1]).sort()).toEqual(["transform", "transform"]);
    expect(framesBody).toMatch(/translateX\(/);
    // animation 系の宣言は、@media (prefers-reduced-motion: no-preference) の中の波（.scene-wave・.wave-N）だけにある
    const mediaBlocks = (query: string) =>
      [...bare.matchAll(new RegExp(`@media \\(prefers-reduced-motion: ${query}\\) \\{`, "g"))].map((m) => topLevelBlock(m.index));
    const [noPref] = mediaBlocks("no-preference");
    expect(mediaBlocks("no-preference")).toHaveLength(1);
    expect(noPref).toContain("wave-drift");
    expect(noPref).toMatch(/\.scene-wave \{[^}]*animation: wave-drift 12s ease-in-out infinite alternate;/);
    for (const m of noPref.matchAll(/^\s*([^{}\n]+) \{/gm)) {
      if (m[1].startsWith("@media")) continue;
      expect(m[1], "動きを付ける選び方").toMatch(/^\.(scene-wave|wave-\d)$/);
    }
    const outside = bare.replace(noPref, "").replace(framesBody, "");
    // 動きを減らす設定では止める。それ以外の所に animation の宣言は無い（止める宣言 none だけ）
    const [reduce] = mediaBlocks("reduce");
    expect(reduce).toContain("animation: none");
    expect(reduce).toMatch(/\.scene-wave \{\s*animation: none;/);
    for (const m of outside.matchAll(/\banimation[a-z-]*:\s*([^;]+);/g)) expect(m[1].trim(), "動きの宣言").toBe("none");
    for (const m of css.matchAll(/url\(\s*["']?([^)"']+)["']?\s*\)/g)) {
      expect(m[1].startsWith("/assets/") || m[1].startsWith("#"), m[1]).toBe(true);
    }
    expect(css).not.toMatch(/@import|https?:\/\//);
  });

  it("押す場所は高さ 44px 以上（ナビ・言語の切り替え・ボタン・文字だけのリンク・リンクの並び・フッター）", () => {
    // 画面の幅や印刷で変わる規則（@media などの中）は除いた、最上位の規則だけを読む。
    // 選び方はコンマで分け、同じ選び方を持つ規則の中身を合わせて見る（書き方を変えても、規則の置き場所が変わっても通る）。
    // min-height は、行の中に並ぶ（inline）要素には効かないので、display も確かめる
    const rules: Array<{ selectors: string[]; body: string }> = [];
    let depth = 0;
    let head = "";
    let body = "";
    let atRule = false;
    for (const ch of css.replace(/\/\*[\s\S]*?\*\//g, "")) {
      if (ch === "{") {
        if (depth === 0) atRule = head.trim().startsWith("@");
        depth++;
        if (depth === 1) continue;
      } else if (ch === "}") {
        depth--;
        if (depth === 0) {
          if (!atRule) rules.push({ selectors: head.split(",").map((s) => s.trim().replace(/\s+/g, " ")), body });
          head = "";
          body = "";
          continue;
        }
      }
      if (depth === 0) head += ch;
      else body += ch;
    }
    const declarations = (sel: string): string => rules.filter((r) => r.selectors.includes(sel)).map((r) => r.body).join("\n");
    for (const sel of [".logo", ".site-nav a", ".lang a", ".lang span", ".button", ".actions a:not(.button)", ".links a", ".site-footer a"]) {
      expect(declarations(sel), `${sel}: min-height: 44px が無い`).toMatch(/min-height: 44px/);
      expect(declarations(sel), `${sel}: display: inline-flex が無い`).toMatch(/display: inline-flex/);
    }
  });

  it("日本語の本文の数字と助数詞の間に空白が無い（サイトの本文・文言とガイド）", () => {
    const files = [...walk(join(ROOT, "site", "content", "ja")), join(ROOT, "site", "lib", "text.ts"), join(ROOT, "site", "lib", "pages.ts"), join(ROOT, "docs", "user", "ja", "guide.md")];
    for (const f of files) {
      for (const [i, line] of readFileSync(f, "utf8").split("\n").entries()) {
        // 「FUSE 2 が要ります」のような製品の版の数字は、助数詞が続かないので当たらない
        expect(line, `${relative(ROOT, f)}:${i + 1}`).not.toMatch(/[0-9] (行|件|つ|か所|営業日|章|世代|時間|桁|字|枚|日|回|個|分|秒|年|月|名|点|種類|人|冊|本)(?![a-zA-Z])/);
      }
    }
  });

  it("日本語のページの英字のラベルには lang=\"en\" が付き、英語のページには付かない", () => {
    const ja = dom(join(plainDir, "ja", "index.html"));
    const en = dom(join(plainDir, "en", "index.html"));
    const labels = (d: Document) => [...d.querySelectorAll("p.eyebrow")];
    expect(labels(ja).length).toBeGreaterThan(0);
    for (const e of labels(ja)) expect(e.getAttribute("lang"), e.textContent ?? "").toBe("en");
    for (const e of labels(en)) expect(e.hasAttribute("lang"), e.textContent ?? "").toBe(false);
  });

  it("印刷用の規則がある（景色とナビを出さない）", () => {
    const start = css.indexOf("@media print {");
    expect(start).toBeGreaterThanOrEqual(0);
    // 印刷の規則は、入れ子の波括弧が 1 段の @media なので、行頭の『}』で終わる
    const block = css.slice(start, css.indexOf("\n}\n", start));
    expect(block).toMatch(/\.skip,\s*\.site-nav,\s*\.hero-scene \{\s*display: none;/);
    expect(block).toMatch(/\.button \{[^}]*background: none/);
    expect(block).toMatch(/pre \{[^}]*white-space: pre-wrap/);
    expect(block).toMatch(/\.doc :is\(p, li\) a\[href\^="http"\]::after,\s*\.site-footer a\[href\^="http"\]::after \{[^}]*content: " \(" attr\(href\) "\)"/);
    expect(block).toMatch(/\.hero,\s*\.hero-sky \{[^}]*background: none/);
  });

  it("ヒーロー：文字は景色（太陽・海・波）に重ならず、波の線は動く幅より外まで描いてある", () => {
    for (const lang of LANGS) {
      const doc = dom(join(plainDir, lang, "index.html"));
      const hero = doc.querySelector("section.hero") as Element;
      const scene = hero.querySelector("svg.hero-scene") as Element;
      // 景色は文字の入れ物（.hero-sky）の外にある兄弟で、中に文字（text・title）を持たない
      expect(scene.parentElement).toBe(hero);
      expect(scene.previousElementSibling?.classList.contains("hero-sky")).toBe(true);
      expect(scene.querySelector("text,title,desc,a,foreignObject")).toBeNull();
      expect(scene.getAttribute("aria-hidden")).toBe("true");
      expect(scene.querySelector("h1,p")).toBeNull();
      expect(hero.querySelector(".hero-sky h1")).not.toBeNull();
      // 景色の縦横比（CSS の aspect-ratio）は viewBox と同じ。狭い画面（幅 360px、高さは最低 200px）で左右を切り取っても、
      // 太陽とその光の道が見える範囲に入る（xMidYMax slice：中央を残して切る）
      const [vbX, vbY, vbWidth, vbHeight] = (scene.getAttribute("viewBox") ?? "").split(" ").map(Number);
      expect([vbX, vbY]).toEqual([0, 0]);
      const ratio = /\.hero-scene \{[^}]*aspect-ratio:\s*(\d+)\s*\/\s*(\d+);/.exec(css);
      expect(ratio, ".hero-scene の aspect-ratio").not.toBeNull();
      expect([Number(ratio?.[1]), Number(ratio?.[2])]).toEqual([vbWidth, vbHeight]);
      const minHeight = Number(/\.hero-scene \{[^}]*min-height:\s*(\d+)px;/.exec(css)?.[1]);
      const scale = Math.max(360 / vbWidth, minHeight / vbHeight);
      const visible = [vbWidth / 2 - 360 / scale / 2, vbWidth / 2 + 360 / scale / 2];
      const sun = scene.querySelector("circle.scene-sun") as Element;
      const sunX = Number(sun.getAttribute("cx"));
      const sunR = Number(sun.getAttribute("r"));
      expect(sunX - sunR, "狭い画面で太陽の左が切れる").toBeGreaterThanOrEqual(visible[0]);
      expect(sunX + sunR, "狭い画面で太陽の右が切れる").toBeLessThanOrEqual(visible[1]);
      // 動くのは波の線だけ。線は左右の端より、CSS の @keyframes が動く幅（translateX の絶対値の最大）を超えて外まで伸びている
      const drift = Math.max(...[...css.matchAll(/translateX\((-?\d+(?:\.\d+)?)px\)/g)].map((m) => Math.abs(Number(m[1]))));
      expect(drift, "@keyframes の translateX が読めない").toBeGreaterThan(0);
      const waves = [...scene.querySelectorAll("path.scene-wave")];
      expect(waves).toHaveLength(3);
      const vbW = Number((scene.getAttribute("viewBox") ?? "").split(" ")[2]);
      for (const w of waves) {
        const d = w.getAttribute("d") ?? "";
        const start = Number(/^M(-?\d+(?:\.\d+)?) /.exec(d)?.[1]);
        // M x y q cx cy dx 0 t dx 0 ...：横の送りの合計が終点
        const steps = [...d.matchAll(/(?:q-?\d+(?:\.\d+)? -?\d+(?:\.\d+)? |t)(\d+(?:\.\d+)?) 0/g)].map((m) => Number(m[1]));
        const end = start + steps.reduce((a, b) => a + b, 0);
        expect(start, d.slice(0, 20)).toBeLessThanOrEqual(-drift);
        expect(end, d.slice(0, 20)).toBeGreaterThanOrEqual(vbW + drift);
      }
    }
  });

  it("ロゴの色は、アプリのロゴ（src/shell/Logo.svelte）と同じ", () => {
    const colors = (s: string) => new Set((s.match(/#[0-9A-Fa-f]{6}|rgba\([^)]+\)/g) ?? []).map((c) => c.toLowerCase()));
    const html = readFileSync(join(plainDir, "ja", "index.html"), "utf8");
    const logo = html.slice(html.indexOf('<svg viewBox="0 0 24 24"'), html.indexOf("</svg>"));
    expect(colors(logo)).toEqual(colors(read("src/shell/Logo.svelte")));
  });

  it("書体は同梱の Inter（可変フォント）で、外部の書体を指さない", () => {
    expect(css).toContain('src: url("/assets/fonts/inter/InterVariable.woff2") format("woff2")');
    expect(css).toMatch(/font-family: Inter, "Hiragino Kaku Gothic ProN", "Yu Gothic", "Noto Sans CJK JP", sans-serif;/);
  });
});

describe("公開のワークフロー（.github/workflows/site.yml）", () => {
  const wf = read(".github/workflows/site.yml");
  /** `  <名前>:` で始まる jobs の中の 1 つのジョブの本文 */
  const job = (name: string): string => {
    const from = wf.indexOf(`\n  ${name}:\n`);
    expect(from, name).toBeGreaterThan(-1);
    const rest = wf.slice(from + 1);
    const next = rest.slice(1).search(/^ {2}[a-z-]+:\n/m);
    return next < 0 ? rest : rest.slice(0, next + 1);
  };
  const code = wf
    .split("\n")
    .filter((l) => !l.trim().startsWith("#"))
    .join("\n");

  it("フォークの PR では動かさない（pull_request 系の契機が無い）。権限は読み取りだけ", () => {
    expect(code).not.toMatch(/pull_request/);
    expect(code).toMatch(/^on:\n {2}push:\n {4}branches: \[main\]/m);
    expect(code).toMatch(/^ {2}workflow_dispatch:/m);
    expect(code).toMatch(/^permissions:\n {2}contents: read\n/m);
    expect(code).not.toMatch(/contents: write|id-token|pull-requests: write/);
  });

  it("uses はすべてコミットの SHA で固定し、版を併記する", () => {
    const uses = [...code.matchAll(/^\s*(?:-\s+)?uses:\s*(\S+)(.*)$/gm)];
    expect(uses.length).toBeGreaterThan(4);
    for (const m of uses) {
      expect(m[1], m[0]).toMatch(/^[\w.-]+\/[\w.-]+@[0-9a-f]{40}$/);
      expect(m[2], m[0]).toMatch(/# v\d+\.\d+\.\d+/);
    }
  });

  it("secrets を使うのは deploy ジョブだけで、そこは Environment と main の条件を持つ。build は secrets を使わない", () => {
    const build = job("build");
    const deploy = job("deploy");
    expect(build).not.toMatch(/secrets\./);
    expect(build).not.toMatch(/environment:/);
    expect(deploy).toMatch(/environment: cloudflare-pages/);
    expect(deploy).toMatch(/if: github\.ref == 'refs\/heads\/main'/);
    expect(deploy).toMatch(/needs: build/);
    // deploy はリポジトリのコードを取り出さず、動かさない（成果物を上げるだけ）
    expect(deploy).not.toMatch(/actions\/checkout|pnpm |node /);
    // 上げる直前に、公開中の latest.json が build で取り込んだものと同じことを確かめる（先に公開されたリリースを戻さない）
    expect(deploy).toMatch(/Check latest\.json is still current[\s\S]+cmp -s site\/dist\/latest\.json[\s\S]+Deploy\n/);
    expect([...code.matchAll(/secrets\.([A-Z_]+)/g)].map((m) => m[1]).sort()).toEqual([
      "CLOUDFLARE_ACCOUNT_ID",
      "CLOUDFLARE_ACCOUNT_ID",
      "CLOUDFLARE_API_TOKEN",
      "CLOUDFLARE_API_TOKEN",
    ]);
  });

  it("自動の公開は変数 SITE_DEPLOY が true のときだけ。wrangler の版を固定し、本番ブランチとプロジェクト名を指す", () => {
    expect(job("build")).toContain("vars.SITE_DEPLOY == 'true'");
    expect(code).toMatch(/wranglerVersion: "\d+\.\d+\.\d+"/);
    expect(code).toContain("pages deploy site/dist --project-name=yunagi --branch=main");
  });

  it("latest.json を取り込む段があり、取れなければ中止する（--allow-missing は手動の実行の入力だけ）", () => {
    expect(code).toContain("node site/fetch-latest.ts --out");
    expect(code).toMatch(/github\.event_name == 'workflow_dispatch' && inputs\.allow_missing_latest/);
    expect(code).toMatch(/--latest-json "\$RUNNER_TEMP\/latest\.json"/);
  });

  it("トリガーのパスに、サイトの元になるファイルが入っている", () => {
    for (const p of ["site/**", "install/install.sh", "install/install.ps1", "scripts/release/package.ts", "scripts/release/version.ts", "docs/user/**", "docs/operations/CHANGELOG.md", "docs/operations/CHANGELOG.en.md"]) {
      expect(code, p).toContain(`- "${p}"`);
    }
  });
});

describe("アプリとそろえる値", () => {
  it("公式サイト・問い合わせ先・Issues の URL は src/lib/support.ts と同じ", () => {
    const support = read("src/lib/support.ts");
    expect(support).toContain(`export const SITE = "${SITE}";`);
    expect(support).toContain(`export const ISSUES_URL = "${ISSUES_URL}";`);
    expect(support).toContain(`export const SUPPORT_MAIL = "${SUPPORT_MAIL}";`);
  });

  it("セキュリティの報告先は SECURITY.md（全文・直下）と同じ", () => {
    const mail = "security@getyunagi.com";
    expect(read("docs/release/SECURITY.md")).toContain(mail);
    expect(read("SECURITY.md")).toContain(mail);
    for (const lang of LANGS) {
      expect(readFileSync(join(plainDir, lang, "security", "index.html"), "utf8")).toContain(`mailto:${mail}`);
    }
  });

  it("導入コマンドは、スクリプトの先頭に書いたものと同じ", () => {
    expect(read("install/install.sh")).toContain(`curl -fsSL ${SITE}/install.sh | sh`);
    expect(read("install/install.ps1")).toContain(`irm ${SITE}/install.ps1 | iex`);
    for (const lang of LANGS) {
      const html = readFileSync(join(plainDir, lang, "index.html"), "utf8");
      expect(html).toContain(`curl -fsSL ${SITE}/install.sh | sh`);
      expect(html).toContain(`irm ${SITE}/install.ps1 | iex`);
    }
  });

  it("法務文書（草案）は、確定版ができるまでサイトに載せない", () => {
    for (const lang of LANGS) {
      for (const f of walk(join(plainDir, lang))) {
        expect(readFileSync(f, "utf8"), f).not.toMatch(/草案|DRAFT — not reviewed/);
      }
    }
    expect(walk(plainDir).some((f) => /terms|privacy|tokushoho/.test(relative(plainDir, f)))).toBe(false);
  });
});
