// 脆弱性の検査の許可リスト（audit-allowlist.json）の検査。
// 決まりは docs/release/07-build-and-distribution.md 3章「許可リスト」。
//
// cargo audit も pnpm audit も、許可した指摘の期限を見ない。そこで、期限の確認はこのスクリプトが受け持つ。
// あわせて、理由と期限の正本（audit-allowlist.json）と、検査の道具が読む設定（.cargo/audit.toml・
// pnpm-workspace.yaml）の ID が食い違っていないことを確かめる。

export type Entry = {
  id: string;
  reason: string;
  remove_when: string;
  expires: string;
};

export type Allowlist = { rust: Entry[]; javascript: Entry[] };

/** 期限は、今日からこの日数までしか先に置けない（置きっぱなしを防ぐ。約 6 か月） */
export const MAX_DAYS_AHEAD = 190;

const RUST_ID = /^RUSTSEC-\d{4}-\d{4}$/;
const GHSA_ID = /^GHSA(-[23456789cfghjmpqrvwx]{4}){3}$/;

/** `YYYY-MM-DD` の日付を、UTC の 0 時の日数にする。実在しない日付は null */
function dayNumber(text: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text);
  if (!m) return null;
  const ms = Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  const back = new Date(ms).toISOString().slice(0, 10);
  return back === text ? ms / 86_400_000 : null;
}

/** 今日（UTC）の日付 `YYYY-MM-DD` */
export function todayUtc(now: Date = new Date()): string {
  return now.toISOString().slice(0, 10);
}

/** 許可リストの形と期限の問題を、人が読む文で返す。無ければ空 */
export function checkAllowlist(list: Allowlist, today: string): string[] {
  const problems: string[] = [];
  const todayNo = dayNumber(today);
  if (todayNo === null) return [`今日の日付が YYYY-MM-DD の形でない: ${today}`];
  const seen = new Set<string>();
  const groups: [string, Entry[], RegExp][] = [
    ["rust", list.rust, RUST_ID],
    ["javascript", list.javascript, GHSA_ID],
  ];
  for (const [kind, entries, idPattern] of groups) {
    for (const e of entries) {
      const name = `${kind} の ${e.id}`;
      if (!idPattern.test(e.id)) problems.push(`${name}: ID の形が違う`);
      if (seen.has(e.id)) problems.push(`${name}: ID が重複している`);
      seen.add(e.id);
      if (!e.reason?.trim()) problems.push(`${name}: 理由（reason）が空`);
      if (!e.remove_when?.trim()) problems.push(`${name}: 外す条件（remove_when）が空`);
      const expiresNo = dayNumber(e.expires ?? "");
      if (expiresNo === null) {
        problems.push(`${name}: 期限（expires）が実在する YYYY-MM-DD の日付でない: ${e.expires}`);
      } else if (expiresNo < todayNo) {
        problems.push(
          `${name}: 期限 ${e.expires} を過ぎている（今日は ${today}）。まだ修正版が無いなら、理由を見直して期限を延ばす。修正版があるなら、依存を上げて外す`,
        );
      } else if (expiresNo - todayNo > MAX_DAYS_AHEAD) {
        problems.push(`${name}: 期限 ${e.expires} が先すぎる（今日から ${MAX_DAYS_AHEAD} 日まで）`);
      }
    }
  }
  return problems;
}

/** `.cargo/audit.toml` の `ignore = [...]` にある ID（コメントの行は読まない） */
export function readCargoIgnore(toml: string): string[] {
  const body = toml
    .split("\n")
    .filter((line) => !line.trimStart().startsWith("#"))
    .join("\n");
  const m = /ignore\s*=\s*\[([^\]]*)\]/.exec(body);
  if (!m) return [];
  return [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]);
}

/** `pnpm-workspace.yaml` の `auditConfig.ignoreGhsas` にある ID（コメントの行は読まない） */
export function readPnpmIgnore(yaml: string): string[] {
  const ids: string[] = [];
  let inList = false;
  for (const raw of yaml.split("\n")) {
    const line = raw.replace(/\s+#.*$/, "");
    if (/^\s*#/.test(line) || line.trim() === "") continue;
    if (/^\s*ignoreGhsas:\s*$/.test(line)) {
      inList = true;
      continue;
    }
    if (inList) {
      const m = /^\s*-\s*(\S+)\s*$/.exec(line);
      if (m) ids.push(m[1]);
      else inList = false;
    }
  }
  return ids;
}

/** 正本の ID と、検査の道具の設定の ID が同じ集まりかを確かめる */
export function checkSync(list: Allowlist, cargoIgnore: string[], pnpmIgnore: string[]): string[] {
  const problems: string[] = [];
  const compare = (label: string, file: string, expected: string[], actual: string[]) => {
    for (const id of expected) {
      if (!actual.includes(id)) problems.push(`${label} の ${id} が ${file} に無い（検査が止まる）`);
    }
    for (const id of actual) {
      if (!expected.includes(id)) {
        problems.push(`${file} の ${id} が audit-allowlist.json に無い（理由と期限が無いまま指摘を通してしまう）`);
      }
    }
  };
  compare("rust", ".cargo/audit.toml", list.rust.map((e) => e.id), cargoIgnore);
  compare("javascript", "pnpm-workspace.yaml", list.javascript.map((e) => e.id), pnpmIgnore);
  return problems;
}
