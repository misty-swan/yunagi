// サイトの文言（ナビ・フッター・トップページ・各ページの説明）。日本語と英語は同じ情報量にそろえる
// （デザインガイド 7章）。短く、淡々と、感嘆符・絵文字・励ましは使わない。
// 長い本文は site/content/<言語>/*.md に書く。
import type { Lang } from "./consts.ts";

export type PageKey = "home" | "download" | "verify" | "guide" | "requirements" | "news" | "support" | "security";

/** ページの URL の末尾（`/<言語>/<slug>/`）。トップは空。 */
export const SLUGS: Record<PageKey, string> = {
  home: "",
  download: "download",
  verify: "verify",
  guide: "guide",
  requirements: "requirements",
  news: "news",
  support: "support",
  security: "security",
};

export const PAGE_KEYS = Object.keys(SLUGS) as PageKey[];

interface Ui {
  skip: string;
  mainNav: string;
  footerNav: string;
  langSwitch: string;
  tableLabel: string;
  nav: Array<{ page: PageKey; label: string }>;
  footerLinks: Array<{ page: PageKey; label: string }>;
  sourceLabel: string;
  licenseNote: string;
  fontNote: string;
  toc: string;
  titleSuffix: string;
}

export const UI: Record<Lang, Ui> = {
  ja: {
    skip: "本文へ移動",
    mainNav: "メイン",
    footerNav: "フッター",
    langSwitch: "言語",
    tableLabel: "表（横にスクロールできます）",
    nav: [
      { page: "download", label: "ダウンロード" },
      { page: "guide", label: "利用者ガイド" },
      { page: "security", label: "セキュリティ" },
      { page: "news", label: "変更履歴・お知らせ" },
    ],
    footerLinks: [
      { page: "verify", label: "配布物の確かめ方" },
      { page: "requirements", label: "動作環境" },
      { page: "support", label: "問い合わせ" },
    ],
    sourceLabel: "ソースコード（GitHub）",
    licenseNote: "本体は MIT / Apache-2.0 のデュアルライセンスです。",
    fontNote: "書体 Inter：SIL Open Font License 1.1",
    toc: "目次",
    titleSuffix: "Yunagi",
  },
  en: {
    skip: "Skip to content",
    mainNav: "Main",
    footerNav: "Footer",
    langSwitch: "Language",
    tableLabel: "Table (scrolls sideways)",
    nav: [
      { page: "download", label: "Download" },
      { page: "guide", label: "Guide" },
      { page: "security", label: "Security" },
      { page: "news", label: "News" },
    ],
    footerLinks: [
      { page: "verify", label: "Verify a download" },
      { page: "requirements", label: "System requirements" },
      { page: "support", label: "Support" },
    ],
    sourceLabel: "Source code (GitHub)",
    licenseNote: "The app is dual-licensed under MIT and Apache-2.0.",
    fontNote: "Typeface Inter: SIL Open Font License 1.1",
    toc: "Contents",
    titleSuffix: "Yunagi",
  },
};

/** ページごとの説明（meta description） */
export const DESCRIPTIONS: Record<Lang, Record<PageKey, string>> = {
  ja: {
    home: "Yunagi は、PC 画面全体を「その日の仕事ノート1枚」として使う、エンジニア向けの日次業務整理アプリです。",
    download: "Yunagi の入れ方。ターミナルから1行で入れる方法、スクリプトを読んでから実行する手順、手動でのダウンロード。",
    verify: "配布物が本物かを、SHA256SUMS と minisign の署名で確かめる手順。",
    guide: "Yunagi の利用者ガイド。インストール、最初の起動、1日の使い方、ショートカット、エージェントとの連携。",
    requirements: "Yunagi の動作環境。対応する OS とアーキテクチャ。",
    news: "Yunagi の変更履歴とお知らせ。",
    support: "Yunagi の不具合報告と問い合わせの窓口。",
    security: "Yunagi のセキュリティ。脆弱性の報告先、対象の版、配布物の保護。",
  },
  en: {
    home: "Yunagi is a daily work-organizing app for engineers. It turns your whole screen into a single note for today's work.",
    download: "How to install Yunagi: the one-line terminal install, reading the script before you run it, and manual downloads.",
    verify: "How to check that a download is genuine, using SHA256SUMS and its minisign signature.",
    guide: "The Yunagi user guide: installing, first launch, a day's use, shortcuts, and working with agents.",
    requirements: "Yunagi system requirements: supported operating systems and architectures.",
    news: "Yunagi change log and announcements.",
    support: "Where to report problems and ask questions about Yunagi.",
    security: "Yunagi security: where to report vulnerabilities, supported versions, and how downloads are protected.",
  },
};

interface Home {
  title: string;
  eyebrow: string;
  heading: [string, string];
  lead: string;
  primary: string;
  secondary: string;
  note: string;
  features: { eyebrow: string; heading: string; cards: Array<{ label: string; title: string; text: string }>; chips: string[] };
  download: {
    eyebrow: string;
    heading: string;
    lead: string;
    mac: string;
    windows: string;
    before: string;
    latest: (version: string) => string;
    links: Array<{ label: string; href: string }>;
  };
  security: {
    eyebrow: string;
    heading: string;
    facts: Array<{ title: string; text: string; href?: string }>;
  };
  guide: { eyebrow: string; heading: string; button: string };
}

export const HOME: Record<Lang, Home> = {
  ja: {
    title: "Yunagi — エンジニア向けの日次業務整理アプリ",
    eyebrow: "Daily work note for engineers",
    heading: ["散らかった1日を、", "静かに見渡せる1枚に。"],
    lead: "Yunagi は、PC 画面全体を「その日の仕事ノート1枚」として使う、エンジニア向けの日次業務整理アプリです。やることを書き出し、終えた仕事が増えるほど、画面の海が静まっていきます。",
    primary: "ダウンロード",
    secondary: "利用者ガイドを読む",
    note: "macOS・Windows・Linux。本体は無料です。",
    features: {
      eyebrow: "Features",
      heading: "書く、見渡す、持ち越す",
      cards: [
        {
          label: "BOARD",
          title: "TODO・障害・振り返りを1枚に",
          text: "1日を3つの領域に分けて並べます。残りの件数は、画面上部の凪メーターが空と海の景色で伝えます。",
        },
        {
          label: "NOTE",
          title: "書いたまま、整理できるノート",
          text: "ノートモードは Vim のキー操作と日本語入力に対応しています。書いた行はそのまま TODO や障害に送れます。",
        },
        {
          label: "AGENTS",
          title: "AI エージェントと同じノートを使う",
          text: "CLI と MCP サーバーから、エージェントが今日の項目を読み書きできます。書き込みは許可した範囲だけで、変更は履歴に残り、1件ずつ取り消せます。",
        },
      ],
      chips: ["持ち越し", "全文検索", "自動保存とバックアップ", "ライト・ダーク", "日本語・英語", "データは手元のファイル"],
    },
    download: {
      eyebrow: "Download",
      heading: "ターミナルから1行で入れる",
      lead: "有料の開発者登録（Apple の署名、Windows のコード署名）は使いません。かわりに、配布物の署名と SHA-256 を公開し、確かめる手順を載せています。",
      mac: "macOS・Linux",
      windows: "Windows（PowerShell）",
      before: "最初のリリースの前です。コマンドは最初のリリースを公開してから動きます。",
      latest: (v) => `最新版：${v}`,
      links: [
        { label: "スクリプトの中身を読んでから実行する手順", href: "download/#2-スクリプトの中身を読んでから実行する" },
        { label: "手動でダウンロードする", href: "download/#4-手動でダウンロードする" },
        { label: "配布物の確かめ方", href: "verify/" },
        { label: "動作環境", href: "requirements/" },
        { label: "アンインストール", href: "guide/#8-アンインストール" },
      ],
    },
    security: {
      eyebrow: "Security",
      heading: "中身を確かめられる",
      facts: [
        { title: "ソースコードは公開", text: "MIT と Apache-2.0 のどちらでも使えます。" },
        {
          title: "配布物の署名と SHA-256",
          text: "SHA256SUMS とその署名（minisign）で、手元で照合できます。",
          href: "verify/",
        },
        {
          title: "データは手元に置く",
          text: "ノートは自分の PC のファイルです。外部への通信は更新の確認（設定で無効にできます）、サポーターライセンスのキーの確認（キーを入力して確かめる操作をしたときだけ）、自分で有効にした機能だけです。",
        },
        {
          title: "脆弱性の報告",
          text: "security@getyunagi.com、または GitHub の非公開の報告で受け付けます。",
          href: "security/",
        },
      ],
    },
    guide: { eyebrow: "Guide", heading: "はじめかたから、エージェントとの連携まで", button: "利用者ガイドを開く" },
  },
  en: {
    title: "Yunagi — a daily work-organizing app for engineers",
    eyebrow: "Daily work note for engineers",
    heading: ["Turn a scattered day", "into one calm page."],
    lead: "Yunagi is a daily work-organizing app for engineers. It uses your whole screen as a single note for today's work. Write down what to do, and the more you finish, the calmer the sea on your screen becomes.",
    primary: "Download",
    secondary: "Read the guide",
    note: "macOS, Windows and Linux. The app is free.",
    features: {
      eyebrow: "Features",
      heading: "Write, look over, carry over",
      cards: [
        {
          label: "BOARD",
          title: "TODOs, blockers and reflection on one page",
          text: "The day is laid out in three areas. The Nagi meter at the top shows how much is left, as a sky and a sea.",
        },
        {
          label: "NOTE",
          title: "A note you can organize as you write",
          text: "Note mode supports Vim keys and Japanese input. A line you wrote can be sent as it is to the TODOs or the blockers.",
        },
        {
          label: "AGENTS",
          title: "Share the same note with AI agents",
          text: "Through the CLI and the MCP server, an agent can read and write today's items. Writing is limited to what you allow, and every change is kept in a history and can be undone one by one.",
        },
      ],
      chips: ["Carry over", "Full-text search", "Autosave and backups", "Light and dark", "Japanese and English", "Your data stays in files on your PC"],
    },
    download: {
      eyebrow: "Download",
      heading: "Install with one line in a terminal",
      lead: "Yunagi does not use paid developer registration (Apple signing, a Windows code-signing certificate). Instead, it publishes signatures and SHA-256 hashes of the downloads, and explains how to check them.",
      mac: "macOS and Linux",
      windows: "Windows (PowerShell)",
      before: "This is before the first release. The commands will work once the first release is published.",
      latest: (v) => `Latest version: ${v}`,
      links: [
        { label: "Read the script before you run it", href: "download/#2-read-the-script-before-you-run-it" },
        { label: "Download manually", href: "download/#4-download-manually" },
        { label: "Verify a download", href: "verify/" },
        { label: "System requirements", href: "requirements/" },
        { label: "Uninstall", href: "guide/#8-uninstall" },
      ],
    },
    security: {
      eyebrow: "Security",
      heading: "You can check what you run",
      facts: [
        { title: "The source code is public", text: "You can use it under either MIT or Apache-2.0." },
        {
          title: "Signature and SHA-256 for every download",
          text: "Check them on your own machine with SHA256SUMS and its minisign signature.",
          href: "verify/",
        },
        {
          title: "Your data stays on your machine",
          text: "Your notes are files on your PC. The only network traffic is the update check (you can turn it off in Settings), the supporter license key check (only when you enter a key and ask Yunagi to verify it), and features you turn on yourself.",
        },
        {
          title: "Reporting a vulnerability",
          text: "Write to security@getyunagi.com, or use GitHub's private reporting.",
          href: "security/",
        },
      ],
    },
    guide: { eyebrow: "Guide", heading: "From first launch to working with agents", button: "Open the guide" },
  },
};
