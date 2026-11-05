// 設定ファイル（TOML）の「文字列の配列」を読む小さな部品。`.cargo/audit.toml` の `ignore = [...]` と
// `about.toml` の `accepted = [...]` に使う。TOML の全部は読まない（この 2 つの形だけ）。

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

/** `<key> = [ "a", "b", ... ]` の文字列を返す（複数行・行末のコメントに対応。無ければ空） */
export function readStringArray(toml: string, key: string): string[] {
  const body = toml.split("\n").map(stripComment).join("\n");
  const m = new RegExp(`^\\s*${key}\\s*=\\s*\\[([^\\]]*)\\]`, "m").exec(body);
  return m ? [...m[1].matchAll(/"([^"]*)"/g)].map((x) => x[1]) : [];
}
