#!/usr/bin/env python3
"""起動時間・メモリ・入力反応を測る（Linux / Hyprland 用。比較用の使い捨て）。

使い方（experiments/yun-2-frontend で）:
    python3 scripts/measure.py startup --runs 10   # 起動時間とメモリ
    python3 scripts/measure.py bench --runs 3      # 入力反応（WebView 内で合成イベント）
    python3 scripts/measure.py listcheck --runs 3  # その場で編集する一覧の自動チェック（合成イベント）

前提: 両版を `pnpm tauri build` 済み（src-tauri/target/release/<名前> があること）。
アプリは Hyprland の見えない特別ワークスペース special:yunagi-bench に開く
（オーナーの画面には出さない。このワークスペースを表示する操作はしない）。

計るもの:
  - spawn → window_ms : Hyprland のイベント（openwindow）が届くまで。ウィンドウが作られた時点
  - spawn → mounted_ms: フロントエンドが画面を組み上げてレイアウトを終え、Rust の `mounted` コマンドが
                        標準出力に MOUNTED を出すまで。描画の直前にあたる
  - spawn → ready_ms  : 最初の描画（requestAnimationFrame 2回）が終わるまで。見えない特別ワークスペースでは
                        描画が起きないので記録されない（2026-11-02 の表示ありの計測でだけ取れている）
  spawn の時刻は hyprctl を呼ぶ直前なので、hyprctl と sh の起動時間（数 ms）を含む
  - pss_mb            : READY から 1.5 秒後の、アプリと子プロセス（WebKit の描画・通信プロセス）の
                        PSS 合計（共有ライブラリを按分したメモリ量）
"""
import argparse
import json
import os
import select
import signal
import socket
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APPS = {
    "svelte": ROOT / "svelte/src-tauri/target/release/yun2-svelte",
    "solid": ROOT / "solid/src-tauri/target/release/yun2-solid",
}
TITLES = {"svelte": "yun-2 Svelte", "solid": "yun-2 Solid"}
RAW = ROOT / "results/raw"


def hypr_event_socket():
    sig = os.environ.get("HYPRLAND_INSTANCE_SIGNATURE")
    if not sig:
        return None
    path = Path(os.environ["XDG_RUNTIME_DIR"]) / "hypr" / sig / ".socket2.sock"
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(str(path))
    s.setblocking(False)
    return s


def descendants(pid):
    children = {}
    for p in Path("/proc").iterdir():
        if not p.name.isdigit():
            continue
        try:
            stat = (p / "stat").read_text()
            ppid = int(stat.rsplit(")", 1)[1].split()[1])
            children.setdefault(ppid, []).append(int(p.name))
        except (OSError, IndexError, ValueError):
            pass
    out, stack = [], [pid]
    while stack:
        cur = stack.pop()
        out.append(cur)
        stack.extend(children.get(cur, []))
    return out


def pss_kb(pid):
    try:
        for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines():
            if line.startswith("Pss:"):
                return int(line.split()[1])
    except OSError:
        pass
    return 0


RUN = ROOT / "results/raw/run"
WORKSPACE = "special:yunagi-bench"


def alive(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] != "Z"
    except OSError:
        return False


def stop(pids):
    """PID を1つずつ指定して止める（広い指定の pkill は使わない）"""
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for p in pids:
            if alive(p):
                try:
                    os.kill(p, sig)
                except ProcessLookupError:
                    pass
        deadline = time.perf_counter() + 5
        while time.perf_counter() < deadline and any(alive(p) for p in pids):
            time.sleep(0.05)
        if not any(alive(p) for p in pids):
            return True
    return not any(alive(p) for p in pids)


def run_once(app, mode, timeout=120.0):
    """見えない特別ワークスペース（special:yunagi-bench）にアプリを開いて測る。
    アプリは hyprctl 経由で起動するので子プロセスではない。出力はログファイル、PID は PID ファイルで受け取る。"""
    RUN.mkdir(parents=True, exist_ok=True)
    log, pidfile = RUN / f"{app}.log", RUN / f"{app}.pid"
    for f in (log, pidfile):
        f.unlink(missing_ok=True)
    inner = f"echo $$ > {pidfile}; exec env YUN2_MODE={mode} {APPS[app]} > {log} 2>/dev/null"
    ev = hypr_event_socket()
    t0 = time.perf_counter()
    subprocess.run(
        # この Hyprland（0.56、Lua 設定）では dispatch に Lua の式を渡す
        ["hyprctl", "dispatch", f'hl.dsp.exec_cmd("sh -c \'{inner}\'", {{ workspace = "{WORKSPACE} silent" }})'],
        check=True, capture_output=True,
    )
    res = {"app": app}
    buf_ev, seen = b"", 0
    deadline = t0 + timeout
    want = "BENCH" if mode in ("bench", "listcheck") else "MOUNTED"
    pid = None
    try:
        while time.perf_counter() < deadline:
            r, _, _ = select.select([ev] if ev else [], [], [], 0.002)
            now = time.perf_counter()
            if ev in r:
                buf_ev += ev.recv(65536)
                while b"\n" in buf_ev:
                    line, buf_ev = buf_ev.split(b"\n", 1)
                    if line.startswith(b"openwindow>>") and TITLES[app].encode() in line and "window_ms" not in res:
                        res["window_ms"] = (now - t0) * 1000
                        res["window_workspace"] = line.decode(errors="replace").split(">>", 1)[1].split(",")[1]
            if pid is None and pidfile.exists():
                txt = pidfile.read_text().strip()
                if txt:
                    pid = int(txt)
            if not log.exists():
                continue
            lines = log.read_text(errors="replace").splitlines()
            # 最後の行は書きかけの可能性があるので、改行まで届いた行だけ読む
            complete = lines if log.read_bytes().endswith(b"\n") else lines[:-1]
            for text in complete[seen:]:
                if text.startswith("SETUP"):
                    res["setup_rust_ms"] = float(text.split("rust_ms=")[1])
                elif text.startswith("MOUNTED"):
                    res["mounted_ms"] = (now - t0) * 1000
                    parts = dict(kv.split("=", 1) for kv in text.split()[1:])
                    res["mounted_rust_ms"] = float(parts["rust_ms"])
                    res["mounted_front_ms"] = float(parts["front_ms"])
                    res["visibility"] = parts["visibility"]
                elif text.startswith("READY"):
                    res["ready_ms"] = (now - t0) * 1000
                    parts = dict(kv.split("=") for kv in text.split()[1:])
                    res["ready_rust_ms"] = float(parts["rust_ms"])
                    res["front_ms"] = float(parts["front_ms"])
                elif text.startswith("BENCH"):
                    res["bench"] = json.loads(text[len("BENCH "):])
            seen = len(complete)
            if want == "MOUNTED" and "mounted_ms" in res and pid:
                time.sleep(1.5)
                procs = descendants(pid)
                res["pss_mb"] = sum(pss_kb(p) for p in procs) / 1024
                res["procs"] = len(procs)
                break
            if want == "BENCH" and "bench" in res:
                break
    finally:
        if pid is None and pidfile.exists() and pidfile.read_text().strip():
            pid = int(pidfile.read_text().strip())
        if pid:
            res["stopped"] = stop(descendants(pid))
        if ev:
            ev.close()
    return res


def checked_run(app, mode):
    """表示先が見えないワークスペースでなかったり、止められなかったりしたら、計測をすぐやめる"""
    r = run_once(app, mode)
    if r.get("window_workspace") != WORKSPACE or not r.get("stopped", False):
        sys.exit(f"中止: 表示先 {r.get('window_workspace')} / 停止 {r.get('stopped')} / {r}")
    return r


def summarize(values):
    values = [v for v in values if v is not None]
    if not values:
        return None
    return {
        "median": round(statistics.median(values), 2),
        "min": round(min(values), 2),
        "max": round(max(values), 2),
        "n": len(values),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("what", choices=["startup", "bench", "listcheck"])
    ap.add_argument("--runs", type=int, default=10)
    args = ap.parse_args()
    RAW.mkdir(parents=True, exist_ok=True)
    for app, path in APPS.items():
        if not path.exists():
            sys.exit(f"{path} がありません。先に pnpm tauri build してください")

    mode = {"bench": "bench", "listcheck": "listcheck"}.get(args.what, "")
    runs = {a: [] for a in APPS}
    # 1回ずつ捨て回し（ディスクキャッシュを温める）。その後は交互に起動して時間帯の偏りを避ける
    for app in APPS:
        checked_run(app, mode)
        time.sleep(1)
    for i in range(args.runs):
        for app in (list(APPS) if i % 2 == 0 else list(reversed(APPS))):
            r = checked_run(app, mode)
            runs[app].append(r)
            print(json.dumps(r, ensure_ascii=False)[:300], flush=True)
            time.sleep(1)

    out = {"what": args.what, "runs": runs, "summary": {}}
    for app, rs in runs.items():
        if args.what == "startup":
            out["summary"][app] = {
                k: summarize([r.get(k) for r in rs])
                for k in ["window_ms", "mounted_ms", "mounted_rust_ms", "mounted_front_ms", "setup_rust_ms", "ready_ms", "pss_mb"]
            }
        elif args.what == "listcheck":
            b = [r["bench"] for r in rs if "bench" in r]
            t = [x["timing"] for x in b if "timing" in x]
            out["summary"][app] = {
                "errors": [x["error"] for x in b if "error" in x],
                "editing_all_runs": [x.get("editing") for x in b],
                "reload_all_runs": [x.get("reload") for x in b],
                **{k: summarize([x[k] for x in t]) for k in (t[0] if t else {})},
            }
        else:
            b = [r["bench"] for r in rs if "bench" in r]
            out["summary"][app] = {
                "add_first100_ms": summarize([x["addMsPerOp"][0] for x in b]),
                "add_last100_ms": summarize([x["addMsPerOp"][-1] for x in b]),
                "add_all_ms": summarize([statistics.mean(x["addMsPerOp"]) for x in b]),
                "toggle_ms": summarize([x["toggleMsPerOp"] for x in b]),
                "clear1000_ms": summarize([x["clearMs"] for x in b]),
                "editor_ms": summarize([x["editorMsPerOp"] for x in b]),
                "ime_logic": [x["ime"] for x in b],
                "userAgent": b[0]["userAgent"] if b else None,
                "visibility": [x.get("visibility") for x in b],
            }
    dest = RAW / f"{args.what}-{time.strftime('%Y%m%d-%H%M%S')}.json"
    dest.write_text(json.dumps(out, ensure_ascii=False, indent=2))
    print(json.dumps(out["summary"], ensure_ascii=False, indent=2))
    print(f"保存しました: {dest}")


if __name__ == "__main__":
    main()
