// 配布用 CI のスクリプトが外部のコマンド（cargo・pnpm・tar）を動かす共通の部品。
import { spawnSync } from "node:child_process";

export type Run = { status: number | null; stdout: string; stderr: string };

/** コマンドを動かして、終了コードと出力を返す（失敗しても例外にしない）。Windows は pnpm などの .cmd のためにシェル経由 */
export function exec(command: string, args: string[]): Run {
  const r = spawnSync(command, args, {
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    shell: process.platform === "win32",
  });
  return { status: r.status, stdout: r.stdout ?? "", stderr: r.error ? String(r.error) : (r.stderr ?? "") };
}

/** コマンドを動かして標準出力を返す。終了コードが 0 でなければ例外にする */
export function execOrThrow(command: string, args: string[]): string {
  const r = exec(command, args);
  if (r.status !== 0) throw new Error(`${command} ${args.join(" ")} が失敗した（終了コード ${r.status}）\n${r.stderr}`);
  return r.stdout;
}
