// ページの外枠（ヘッダー・フッター・言語の切り替え）と、景色の図。
// 外部の読み込み・スクリプト・インラインの style は使わない（_headers の CSP が許さない）。
import { ISSUES_URL, REPO, SITE, SUPPORT_MAIL, LANGS, type Lang } from "./consts.ts";
import { esc } from "./html.ts";
import { SLUGS, UI, type PageKey } from "./text.ts";

/** ページの URL（`/<言語>/<slug>/`。トップは `/<言語>/`）。fragment は付けない。 */
export function pagePath(lang: Lang, page: PageKey): string {
  const slug = SLUGS[page];
  return slug === "" ? `/${lang}/` : `/${lang}/${slug}/`;
}

/** ロゴの絵（src/shell/Logo.svelte と同じ図形と色。テーマで変えない。site/site.test.ts が色を照合する） */
const LOGO_SVG = `<svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
<defs>
<linearGradient id="logo-sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2B2542"/><stop offset="0.34" stop-color="#4A3550"/><stop offset="0.54" stop-color="#D88A5E"/><stop offset="0.58" stop-color="#EFA567"/></linearGradient>
<linearGradient id="logo-sea" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#1A2236"/><stop offset="1" stop-color="#0F1526"/></linearGradient>
<clipPath id="logo-round"><rect width="24" height="24" rx="6"/></clipPath>
<clipPath id="logo-above"><rect width="24" height="13.92"/></clipPath>
</defs>
<g clip-path="url(#logo-round)">
<rect width="24" height="13.92" fill="url(#logo-sky)"/>
<rect y="13.92" width="24" height="10.08" fill="url(#logo-sea)"/>
<circle cx="12" cy="15.4" r="4.32" fill="#F6B57A" clip-path="url(#logo-above)"/>
<rect x="3.6" y="13.92" width="16.8" height="1" fill="rgba(255,236,220,0.85)"/>
</g>
</svg>`;

/** 景色の大きさ（viewBox）と水平線・太陽の位置。文字は景色の上に載せない（コピーは景色の上の空に置く） */
const SCENE = { w: 1280, h: 320, horizon: 190, sunX: 760 } as const;
/** 波が横に動く幅（site.css の @keyframes の 12px の 2 倍）より外まで、線を左右に伸ばす（端の切れ目を見せない） */
const WAVE_OVERSHOOT = 160;

/** 波の線：半周期 half の山と谷を、左右に WAVE_OVERSHOOT ずつ余分に伸ばして並べる */
function wavePath(y: number, half: number, bend: number): string {
  const from = -WAVE_OVERSHOOT;
  const count = Math.ceil((SCENE.w + WAVE_OVERSHOOT * 2) / half);
  return `M${from} ${y}q${half / 2} ${-bend} ${half} 0${`t${half} 0`.repeat(count - 1)}`;
}

/**
 * ヒーローの景色（凪メーターと同じ空・太陽・光の道・海・波）。色は CSS のトークン。
 * 波の線だけが動く（site.css の wave-drift。prefers-reduced-motion では止まる）。
 */
export const SCENE_SVG = `<svg class="hero-scene" viewBox="0 0 ${SCENE.w} ${SCENE.h}" preserveAspectRatio="xMidYMax slice" aria-hidden="true" focusable="false">
<defs>
<linearGradient id="scene-sky" x1="0" y1="0" x2="0" y2="1"><stop class="sky-top" offset="0"/><stop class="sky-bottom" offset="${(SCENE.horizon / SCENE.h).toFixed(3)}"/><stop class="sky-bottom" offset="1"/></linearGradient>
<radialGradient id="scene-glow"><stop class="glow-stop" offset="0"/><stop class="glow-end" offset="1"/></radialGradient>
</defs>
<rect class="scene-sky" width="${SCENE.w}" height="${SCENE.h}"/>
<circle class="scene-glow" cx="${SCENE.sunX}" cy="${SCENE.horizon}" r="140"/>
<circle class="scene-halo" cx="${SCENE.sunX}" cy="${SCENE.horizon}" r="88"/>
<circle class="scene-sun" cx="${SCENE.sunX}" cy="${SCENE.horizon}" r="44"/>
<rect class="scene-sea" y="${SCENE.horizon}" width="${SCENE.w}" height="${SCENE.h - SCENE.horizon}"/>
<rect class="scene-horizon" y="${SCENE.horizon - 1}" width="${SCENE.w}" height="1.5"/>
<rect class="scene-path" x="${SCENE.sunX - 30}" y="${SCENE.horizon + 4}" width="60" height="3" opacity="0.8"/>
<rect class="scene-path" x="${SCENE.sunX - 20}" y="${SCENE.horizon + 16}" width="40" height="3" opacity="0.6"/>
<rect class="scene-path" x="${SCENE.sunX - 12}" y="${SCENE.horizon + 28}" width="24" height="3" opacity="0.45"/>
<rect class="scene-path" x="${SCENE.sunX - 7}" y="${SCENE.horizon + 40}" width="14" height="3" opacity="0.3"/>
<path class="scene-wave wave-1" opacity="0.5" d="${wavePath(252, 80, 8)}"/>
<path class="scene-wave wave-2" opacity="0.4" d="${wavePath(280, 120, 10)}"/>
<path class="scene-wave wave-3" opacity="0.3" d="${wavePath(306, 160, 12)}"/>
</svg>`;

/** どのページの <head> にも入れる共通の部分（色の方式・アイコン・CSS）。トップの移動ページと 404 も同じものを使う */
export const HEAD_COMMON = `<meta name="color-scheme" content="light dark">
<link rel="icon" href="/assets/favicon.svg" type="image/svg+xml">
<link rel="stylesheet" href="/assets/site.css">
`;

export interface PageInput {
  lang: Lang;
  page: PageKey;
  title: string;
  description: string;
  body: string;
  /** 本文の外枠の class。トップは区画ごとの余白を自分で持つので "" */
  mainClass?: string;
}

const LANG_NAMES: Record<Lang, string> = { ja: "日本語", en: "English" };

export function renderPage(input: PageInput): string {
  const { lang, page, title, description, body } = input;
  const ui = UI[lang];
  const other = LANGS.find((l) => l !== lang) as Lang;
  const url = (l: Lang) => `${SITE}${pagePath(l, page)}`;
  const fullTitle = page === "home" ? title : `${title} — ${ui.titleSuffix}`;

  const nav = ui.nav
    .map((n) => {
      const href = pagePath(lang, n.page);
      const current = n.page === page ? ' aria-current="page"' : "";
      return `<a href="${href}"${current}>${esc(n.label)}</a>`;
    })
    .join("\n");

  const switcher = LANGS.map((l) =>
    l === lang
      ? `<span lang="${l}" aria-current="true">${LANG_NAMES[l]}</span>`
      : `<a href="${pagePath(l, page)}" hreflang="${l}" lang="${l}">${LANG_NAMES[l]}</a>`,
  ).join("");

  const footerLinks = [
    ...ui.footerLinks.map((n) => `<li><a href="${pagePath(lang, n.page)}">${esc(n.label)}</a></li>`),
    `<li><a href="${REPO}">${esc(ui.sourceLabel)}</a></li>`,
    `<li><a href="${ISSUES_URL}">GitHub Issues</a></li>`,
    `<li><a href="mailto:${SUPPORT_MAIL}">${SUPPORT_MAIL}</a></li>`,
  ].join("\n");

  return `<!doctype html>
<html lang="${lang}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(fullTitle)}</title>
<meta name="description" content="${esc(description)}">
${HEAD_COMMON}<link rel="canonical" href="${url(lang)}">
<link rel="alternate" hreflang="${lang}" href="${url(lang)}">
<link rel="alternate" hreflang="${other}" href="${url(other)}">
<link rel="alternate" hreflang="x-default" href="${url("ja")}">
</head>
<body>
<a class="skip" href="#main">${esc(ui.skip)}</a>
<header class="site-header">
<div class="wrap">
<a class="logo" href="${pagePath(lang, "home")}">${LOGO_SVG}<span>yunagi</span></a>
<nav class="site-nav" aria-label="${esc(ui.mainNav)}">
${nav}
<span class="lang" role="group" aria-label="${esc(ui.langSwitch)}">${switcher}</span>
</nav>
</div>
</header>
<main id="main"${input.mainClass ? ` class="${input.mainClass}"` : ""}>
${body}
</main>
<footer class="site-footer">
<div class="wrap">
<div>
<p class="brand">yunagi</p>
<p>${esc(ui.licenseNote)}</p>
<p><a href="/assets/fonts/inter/LICENSE.txt">${esc(ui.fontNote)}</a></p>
</div>
<nav aria-label="${esc(ui.footerNav)}">
<ul>
${footerLinks}
</ul>
</nav>
</div>
</footer>
</body>
</html>
`;
}
