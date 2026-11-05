// 設定ファイル（TOML）の「文字列の配列」を読む小さな部品。`.cargo/audit.toml` の `[advisories]` の `ignore = [...]` と
// `about.toml` の先頭の `accepted = [...]` に使う。TOML の全部は読まない（この形だけ）。

/** 行の `#` から後ろ（文字列の外のもの）を取り除く */
function stripComment(line: string): string {
  let inString = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === '"') inString = !inString;
    else if (c === "#" && !inString) return line.slice(0, i);
  }
  return line;
}

/** 行ごとに、その行がある表（`[advisories]` の `advisories`。表の外は空文字列）を付ける */
function withTables(toml: string): { table: string; text: string }[] {
  let table = "";
  return toml.split("\n").map((raw) => {
    const text = stripComment(raw);
    const header = /^\s*\[([^\]]+)\]\s*$/.exec(text);
    if (header) table = header[1].trim();
    return { table, text };
  });
}

/**
 * `<key> = [ "a", "b", ... ]` の文字列を返す（複数行・行末のコメントに対応）。`table` の中のものだけを読む
 * （表の外は空文字列）。無ければ空。
 */
export function readStringArray(toml: string, key: string, table = ""): string[] {
  const body = withTables(toml)
    .filter((l) => l.table === table)
    .map((l) => l.text)
    .join("\n");
  const m = new RegExp(`^\\s*${key}\\s*=\\s*\\[([^\\]]*)\\]`, "m").exec(body);
  return m ? [...m[1].matchAll(/"([^"]*)"/g)].map((x) => x[1]) : [];
}

/** `<key> = [ ... ]`（表の外）を、新しい文字列の配列に置き換える。無ければそのまま返す */
export function replaceStringArray(toml: string, key: string, values: readonly string[]): string {
  const list = values.map((v) => `    "${v}",\n`).join("");
  return toml.replace(new RegExp(`^${key}\\s*=\\s*\\[[^\\]]*\\]`, "m"), `${key} = [\n${list}]`);
}
