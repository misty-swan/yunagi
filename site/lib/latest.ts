// latest.json（更新情報。ビルド・配布手順書 5.1）の読み取りと検査。
// サイトのビルドと、公開用のワークフローが「いま公開中の latest.json」を取り込む段が使う。
import { VERSION_PATTERN } from "../../scripts/release/version.ts";

// 形は Tauri の静的な形（version・notes・pub_date・platforms）。インストールスクリプトは
// ファイルの中で最初に現れる "version" の値を読むので、トップレベルの version を入れ子の項目より前に置く（4.1 の 2）。

/** サイトに出す分だけの読み取り結果 */
export interface Latest {
  version: string;
  pubDate: string | null;
  /** 公開する配布物の組（`platforms` のキー。ダウンロードのページの一覧は、これにある組だけを出す） */
  platforms: string[];
}

const PLATFORMS = [
  "darwin-aarch64",
  "darwin-x86_64",
  "linux-x86_64",
  "linux-aarch64",
  "windows-x86_64",
  "windows-aarch64",
];

/** 形が正しくなければ、理由を付けて例外を投げる。 */
export function parseLatest(text: string): Latest {
  let json: unknown;
  try {
    json = JSON.parse(text);
  } catch {
    throw new Error("latest.json が JSON として読めない");
  }
  if (typeof json !== "object" || json === null || Array.isArray(json)) {
    throw new Error("latest.json のトップレベルがオブジェクトではない");
  }
  const obj = json as Record<string, unknown>;
  const version = obj.version;
  if (typeof version !== "string" || !VERSION_PATTERN.test(version)) {
    throw new Error(`latest.json の version が <数>.<数>.<数> の形ではない: ${String(version)}`);
  }
  // インストールスクリプトは最初の "version" を読む。ほかの項目より前に置く
  const first = text.search(/"version"\s*:/);
  const platformsAt = text.search(/"platforms"\s*:/);
  if (first < 0 || (platformsAt >= 0 && platformsAt < first)) {
    throw new Error('latest.json の "version" が "platforms" より後ろにある（インストールスクリプトが読み違える）');
  }
  const platforms = obj.platforms;
  if (typeof platforms !== "object" || platforms === null || Array.isArray(platforms)) {
    throw new Error("latest.json の platforms がオブジェクトではない");
  }
  const entries = Object.entries(platforms as Record<string, unknown>);
  if (entries.length === 0) throw new Error("latest.json の platforms が空");
  for (const [key, value] of entries) {
    if (!PLATFORMS.includes(key)) throw new Error(`latest.json の platforms に未知のキー: ${key}`);
    const v = value as Record<string, unknown> | null;
    if (typeof v !== "object" || v === null || typeof v.signature !== "string" || v.signature === "") {
      throw new Error(`latest.json の platforms.${key} に signature が無い`);
    }
    if (typeof v.url !== "string" || !v.url.startsWith("https://")) {
      throw new Error(`latest.json の platforms.${key} の url が https ではない`);
    }
  }
  const pubDate = typeof obj.pub_date === "string" ? obj.pub_date : null;
  return { version, pubDate, platforms: entries.map(([key]) => key) };
}

/** 取得の結果を、公開用のワークフローが取るべき動きに直す。 */
export type FetchedLatest =
  | { kind: "ok"; text: string; latest: Latest }
  | { kind: "missing"; reason: string }
  | { kind: "error"; reason: string };

/** 取得の試み。通信が止まったままにならないよう時間切れを付け、一時的な失敗は数回やり直す。 */
export interface FetchOptions {
  fetchImpl?: typeof fetch;
  /** 1 回の取得の時間切れ（ミリ秒） */
  timeoutMs?: number;
  /** 試す回数の上限 */
  attempts?: number;
  /** やり直すまでの待ち（ミリ秒） */
  delayMs?: number;
}

/**
 * いま公開中の latest.json を取る（サイトだけの変更のデプロイが、これを含める。ビルド・配布手順書 3.4）。
 * 404 は「まだ公開していない」、それ以外の失敗（通信できない・5xx・形の誤り）は「取れなかった」で分ける。
 * 通信の失敗と 5xx は、時間切れを付けて数回やり直す（一時的な失敗で公開全体を止めないため）。404 と形の誤りはやり直さない。
 * 取れなかったときに、含め忘れて 404 にしない・古い写しで上書きしないのは、呼ぶ側が決める。
 */
export async function fetchPublishedLatest(url: string, options: FetchOptions = {}): Promise<FetchedLatest> {
  const { fetchImpl = fetch, timeoutMs = 15_000, attempts = 3, delayMs = 2_000 } = options;
  let last: FetchedLatest = { kind: "error", reason: "取得を試していない" };
  for (let i = 0; i < attempts; i++) {
    if (i > 0) await new Promise((r) => setTimeout(r, delayMs));
    let res: Response;
    try {
      // キャッシュに頼らず、配布元から取る
      res = await fetchImpl(url, {
        headers: { "Cache-Control": "no-cache" },
        redirect: "error",
        signal: AbortSignal.timeout(timeoutMs),
      });
    } catch (e) {
      last = { kind: "error", reason: `取得できなかった: ${e instanceof Error ? e.message : String(e)}` };
      continue;
    }
    if (res.status === 404) return { kind: "missing", reason: `${url} は 404（まだ公開していない）` };
    if (res.status >= 500) {
      last = { kind: "error", reason: `${url} が ${res.status} を返した` };
      continue;
    }
    if (!res.ok) return { kind: "error", reason: `${url} が ${res.status} を返した` };
    const text = await res.text();
    try {
      return { kind: "ok", text, latest: parseLatest(text) };
    } catch (e) {
      return { kind: "error", reason: e instanceof Error ? e.message : String(e) };
    }
  }
  return last;
}
