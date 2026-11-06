// @vitest-environment node
// 利用者ガイド（日英）の検査：日英の章立てと中身がそろっていること、ガイドに書いたキー・コマンド・画面の
// 文言が実際のコードと食い違っていないこと、設計書の内部の言葉が出ていないこと。
// 決まりは docs/release/08-test-plan.md 4.12。ガイドは docs/user/ja/guide.md と docs/user/en/guide.md。
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (path: string): string => readFileSync(join(ROOT, path), "utf8");

const GUIDES = { ja: read("docs/user/ja/guide.md"), en: read("docs/user/en/guide.md") };

/** コードの囲み（```）の外の行だけ。囲みの中の `#` を見出しと読まないため。 */
function proseLines(text: string): string[] {
  const out: string[] = [];
  let fenced = false;
  for (const line of text.split("\n")) {
    if (line.startsWith("```")) {
      fenced = !fenced;
      continue;
    }
    if (!fenced) out.push(line);
  }
  return out;
}

/** 見出しの深さと、先頭の番号（`1.` など。無ければ空）。 */
function headings(text: string): string[] {
  return proseLines(text)
    .map((l) => /^(#{1,6}) (.*)$/.exec(l))
    .filter((m): m is RegExpExecArray => m !== null)
    .map((m) => `${m[1]} ${/^(\d+)\./.exec(m[2])?.[1] ?? ""}`);
}

/** コードの囲みごとの言語名と行数。 */
function fences(text: string): string[] {
  const out: string[] = [];
  let lang: string | null = null;
  let lines = 0;
  for (const line of text.split("\n")) {
    if (!line.startsWith("```")) {
      if (lang !== null) lines += 1;
      continue;
    }
    if (lang === null) {
      lang = line.slice(3).trim();
      lines = 0;
    } else {
      out.push(`${lang}:${lines}`);
      lang = null;
    }
  }
  return out;
}

/** 表の行（先頭が `|` の行）。区切りの行は除く。 */
function tableRows(text: string): string[][] {
  return proseLines(text)
    .filter((l) => l.startsWith("|") && !/^\|[\s|-]+\|$/.test(l))
    .map((l) => {
      const cols = l
        .slice(1, -1)
        .split(" | ")
        .map((c) => c.trim().replace(/^\||\|$/g, "").trim());
      if (cols.length !== 2) throw new Error(`表の行が 2 列ではありません: ${l}`);
      return cols;
    });
}

const KEY = /\b(?:(?:Mod|Ctrl|Alt|Shift)\+)+(?:Enter|Backspace|Delete|Tab|Space|Esc|[A-Za-z0-9]|\\\\|[[\],↑↓←→])(?![A-Za-z0-9])/g;

/** 表のキーの欄を、割当の文字列（`Mod+Shift+L` など。キーの名前は keymap.ts と同じ）の並びにする。 */
function tableKeys(text: string): string[][] {
  return tableRows(text)
    .slice(1) // 見出しの行（表は 1 つだけ。上の試験で数える）
    .map((cols) =>
      (cols[1].replace(/[（(][^）)]*[）)]/g, "").match(KEY) ?? []).map((k) =>
        k.replace("\\\\", "\\").replace("↑", "ArrowUp").replace("↓", "ArrowDown"),
      ),
    );
}

describe("日英の構成がそろっている", () => {
  it("見出しの数・深さ・章の番号が同じ順で並ぶ", () => {
    expect(headings(GUIDES.en)).toEqual(headings(GUIDES.ja));
    expect(headings(GUIDES.ja).filter((h) => h.startsWith("## ")).length).toBe(8);
  });

  it("コードの囲みの数・言語・行数が同じ順で並ぶ", () => {
    expect(fences(GUIDES.en)).toEqual(fences(GUIDES.ja));
  });

  it("表の行数と、キーの欄が同じ", () => {
    const ja = tableRows(GUIDES.ja);
    const en = tableRows(GUIDES.en);
    expect(en.length).toBe(ja.length);
    for (const t of [GUIDES.ja, GUIDES.en]) expect(proseLines(t).filter((l) => /^\|[\s|-]+\|$/.test(l)).length).toBe(1);
    expect(tableKeys(GUIDES.en)).toEqual(tableKeys(GUIDES.ja));
  });

  it("文中のキー（Mod+K など）が同じ順で出る", () => {
    const keys = (t: string) => proseLines(t).join("\n").match(KEY);
    expect(keys(GUIDES.en)).toEqual(keys(GUIDES.ja));
  });

  it("「要確認」の但し書きが、日英で同じ数だけ残っている（公開前に消す。docs/release/07-build-and-distribution.md 4.3・5章）", () => {
    const count = (t: string, s: string) => t.split(s).length - 1;
    expect(count(GUIDES.en, "to be confirmed")).toBe(count(GUIDES.ja, "要確認"));
  });

  it("公開前に決まる仮の表記（tap の名前）が、日英で同じ数だけ残っている", () => {
    const count = (t: string, s: string) => t.split(s).length - 1;
    expect(count(GUIDES.en, "<tap>")).toBe(count(GUIDES.ja, "<tap名>"));
  });

  it("公式サイトの仮の表記は残っておらず、導入コマンドは本物の公式サイト（getyunagi.com）を指す", () => {
    for (const t of [GUIDES.ja, GUIDES.en]) {
      expect(t).not.toMatch(/<公式サイト>|<official-site>/);
      expect(t).toContain("curl -fsSL https://getyunagi.com/install.sh | sh");
      expect(t).toContain("irm https://getyunagi.com/install.ps1 | iex");
      expect(t).toContain("curl -fsSL https://getyunagi.com/install.sh | YUNAGI_VERSION=1.2.3 sh");
      expect(t).toContain('$env:YUNAGI_VERSION="1.2.3"; irm https://getyunagi.com/install.ps1 | iex');    }
  });
});

describe("コードと食い違っていない", () => {
  const keymap = read("src/lib/keymap.ts");
  const initials = [...keymap.matchAll(/initial: ("[^"]+")/g)].map((m) => JSON.parse(m[1]) as string);

  it("ショートカットの表が、keymap.ts の初期割当と一致する（取り消し・やり直しを除く）", () => {
    expect(initials.length).toBeGreaterThan(15);
    const fixed = new Set(["Mod+Z", "Mod+Shift+Z"]);
    for (const text of [GUIDES.ja, GUIDES.en]) {
      const keys = tableKeys(text).flat();
      for (const k of keys) expect(initials.includes(k) || fixed.has(k), k).toBe(true);
      for (const k of initials) expect(keys, k).toContain(k);
    }
  });

  // `yunagi --help` の本文（args.rs の `USAGE`）
  const usageMatch = /pub const USAGE: &str = "\\\n([\s\S]*?)";\n/.exec(read("crates/cli/src/args.rs"));
  if (usageMatch === null) throw new Error("crates/cli/src/args.rs の USAGE が読めません（書き方を変えたなら、この試験の読み方も直す）");
  const usage = usageMatch[1];

  it("ガイドのコマンドとオプションが、yunagi --help の一覧にある", () => {
    for (const text of [GUIDES.ja, GUIDES.en]) {
      const spans = [...text.matchAll(/`([^`\n]*)`/g)].map((m) => m[1]);
      const prose = new Set(proseLines(text));
      const codeLines = text
        .split("\n")
        .filter((l) => !prose.has(l)) // コードの囲みの中の行だけ
        .filter((l) => /(?<![/\w.-])yunagi [a-z]/.test(l))
        .map((l) => l.replace(/^.*?(?<![/\w.-])(yunagi [a-z].*)$/, "$1"));
      const commands = [...spans, ...codeLines].filter((s) => /^yunagi [a-z]/.test(s));
      expect(commands.length).toBeGreaterThan(8);
      for (const c of commands) {
        const [, first, second] = /^yunagi ([a-z]+)(?: ([a-z]+))?/.exec(c)!;
        const lines = usage.split("\n").filter((l) => new RegExp(`^  ${first}\\b`).test(l));
        expect(lines.length, c).toBeGreaterThan(0);
        if (second !== undefined) {
          expect(lines.some((l) => new RegExp(`^  ${first} \\[?${second}\\b`).test(l)), c).toBe(true);
        }
        // オプションは、そのコマンドの行にあるものだけ
        for (const o of c.match(/(?<![\w-])--[a-z][a-z-]*/g) ?? []) {
          expect(lines.some((l) => new RegExp(`${o}(?![\\w-])`).test(l)), `${c} の ${o}`).toBe(true);
        }
      }
      expect(new Set(commands.flatMap((c) => c.match(/(?<![\w-])--[a-z][a-z-]*/g) ?? [])).size).toBeGreaterThan(3);
    }
  });
});

describe("画面の文言がコードにある", () => {
  const code = ["src/lib/i18n.ts", "src/lib/i18n_settings.ts", "src/lib/i18n_update.ts", "src/lib/i18n_agents.ts"]
    .map(read)
    .join("\n");
  // [日本語, English]：どちらもガイドの該当する言語に出て、コードの文言にも同じ文字列である
  const LABELS: ReadonlyArray<readonly [string, string]> = [
    ["ショートカット割当", "Keyboard shortcuts"],
    ["Vim モード", "Vim mode"],
    ["凪メーターの時間帯", "Nagi meter hours"],
    ["更新の自動確認", "Check for updates automatically"],
    ["挿入モードで入力状態を戻す", "Restore input state in Insert mode"],
    ["New version", "New version"],
    ["CARRY OVER", "CARRY OVER"],
    ["更新できませんでした", "Couldn't update"],
    ["言語", "Language"],
    ["選んだものを持ち越す", "Carry over selected"],
    ["すべて持ち越す", "Carry over all"],
    ["持ち越さない", "Don’t carry over"],
    ["分割する", "Split"],
    ["やめる", "Drop"],
    ["持ち越しを開く", "Open carry-over"],
    ["更新して再起動", "Update and restart"],
    ["保存できていません", "Not saved"],
  ];

  it("ガイドの設定の項目名・ボタン名・通知が、画面の文言と同じ", () => {
    for (const [ja, en] of LABELS) {
      // 文言の全体か、その後に文が続くもの（「更新できませんでした。…」）
      expect(code, ja).toMatch(new RegExp(`"${ja}(?:"|。)`));
      expect(code, en).toMatch(new RegExp(`"${en}(?:"|\\.)`));
      expect(GUIDES.ja, ja).toContain(ja);
      expect(GUIDES.en, en).toContain(en);
    }
  });

  it("設定の区分の名前は、日英とも英字（General・Display・Operation・Data・Agents）で書く", () => {
    for (const text of [GUIDES.ja, GUIDES.en]) {
      for (const bad of ["「一般」", "「操作」", "「データ」", "「エージェント」", "Settings → Input", "→ General", "→ Data"]) {
        expect(text, bad).not.toContain(bad);
      }
    }
    for (const section of ["General", "Display", "Operation", "Data", "Agents"]) {
      expect(code).toContain(`${section.toLowerCase()}: "${section}"`);
      expect(GUIDES.ja).toContain(`「${section}」区分`);
      expect(GUIDES.en).toContain(`${section} section`);
    }
    for (const text of [GUIDES.ja, GUIDES.en]) {
      expect(text).not.toMatch(/「(?:一般|表示|操作|データ|エージェント)」区分/);
      expect(text).not.toMatch(/Settings\s*(?:→|>)\s*Input/);
    }
  });
});

describe("利用者向けの書き方", () => {
  it("設計書の内部の言葉（課題番号・設計書のパス・章の参照）を出さない", () => {
    for (const text of [GUIDES.ja, GUIDES.en]) {
      expect(text).not.toMatch(/YUN-\d+/);
      expect(text).not.toMatch(/docs\//);
      expect(text).not.toMatch(/設計書|仕様書|手順書|テスト計画/);
      expect(text).not.toMatch(/\b(?:design doc|spec|test plan)\b/i);
    }
  });
});
