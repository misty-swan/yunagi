// @vitest-environment node
// インストールスクリプト（install.sh・install.ps1）の試験。
// 利用者の環境（~/.local など）には触れない：HOME・XDG_* は一時フォルダにし、`uname` は偽物に差し替え、
// 配布物は一時フォルダに作った偽の配布元から `file://` で取る（ネットには出ない）。
// 方式は docs/release/07-build-and-distribution.md 4.5。
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readlinkSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

const SCRIPT = resolve(__dirname, "install.sh");
const PS1 = resolve(__dirname, "install.ps1");
const VERSION = "1.2.3";

let root: string; // 1 件ごとの一時フォルダ
let home: string;
let dist: string; // 偽の配布元（<dist>/v1.2.3/ に配布物を置く）
let bin: string; // 偽の uname を置く

function sha256(data: Buffer | string): string {
  return createHash("sha256").update(data).digest("hex");
}

function put(name: string, data: Buffer | string, hash: string = sha256(data)) {
  const dir = join(dist, `v${VERSION}`);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, name), data);
  writeFileSync(join(dir, `${name}.sha256`), `${hash}  ${name}\n`);
}

function tarGz(files: Record<string, string>): Buffer {
  const src = mkdtempSync(join(root, "tar-"));
  for (const [path, body] of Object.entries(files)) {
    mkdirSync(join(src, dirname(path)), { recursive: true });
    writeFileSync(join(src, path), body, { mode: 0o755 });
  }
  const out = join(src, "out.tar.gz");
  const r = spawnSync("tar", ["-czf", out, "-C", src, ...Object.keys(files).map((f) => f.split("/")[0])]);
  expect(r.status).toBe(0);
  return readFileSync(out);
}

function stockRelease(arch: string) {
  put(`Yunagi_${VERSION}_${arch}.AppImage`, "appimage-bytes");
  put(`yunagi-cli_${VERSION}_${arch}.tar.gz`, tarGz({ yunagi: "#!/bin/sh\necho cli\n" }));
  put(
    `Yunagi_${VERSION}_${arch}.app.tar.gz`,
    tarGz({ "Yunagi.app/Contents/MacOS/yunagi": "#!/bin/sh\necho cli\n" }),
  );
}

type Result = { status: number | null; out: string; err: string };

function run(env: Record<string, string> = {}, uname = { s: "Linux", m: "x86_64" }, viaStdin = false): Result {
  const fullEnv: Record<string, string> = {
    PATH: `${bin}:${process.env.PATH ?? ""}`,
    HOME: home,
    TMPDIR: root,
    FAKE_UNAME_S: uname.s,
    FAKE_UNAME_M: uname.m,
    YUNAGI_BASE_URL: `file://${dist}`,
    YUNAGI_VERSION: VERSION,
    ...env,
  };
  // XDG_* と WAYLAND_DISPLAY は、指定したものだけにする（試験を動かす人の環境を引き継がない）
  const r = viaStdin
    ? spawnSync("sh", [], { env: fullEnv, input: readFileSync(SCRIPT), encoding: "utf8" })
    : spawnSync("sh", [SCRIPT], { env: fullEnv, encoding: "utf8" });
  return { status: r.status, out: r.stdout, err: r.stderr };
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "yunagi-install-test-"));
  home = join(root, "home");
  dist = join(root, "dist");
  bin = join(root, "bin");
  mkdirSync(home);
  mkdirSync(bin);
  writeFileSync(
    join(bin, "uname"),
    '#!/bin/sh\ncase "$1" in -s) echo "$FAKE_UNAME_S" ;; -m) echo "$FAKE_UNAME_M" ;; *) echo "$FAKE_UNAME_S" ;; esac\n',
    { mode: 0o755 },
  );
});

afterEach(() => {
  rmSync(root, { recursive: true, force: true });
});

describe("install.sh：Linux", () => {
  it("AppImage と CLI を置き、リンクとデスクトップエントリを作り、置いた場所を一覧にする", () => {
    stockRelease("x86_64");
    const r = run({ XDG_SESSION_TYPE: "x11" });
    expect(r.err).toBe("");
    expect(r.status).toBe(0);

    const app = join(home, ".local/opt/yunagi/Yunagi.AppImage");
    const cli = join(home, ".local/opt/yunagi/bin/yunagi");
    expect(readFileSync(app, "utf8")).toBe("appimage-bytes");
    expect(statSync(app).mode & 0o111).not.toBe(0);
    expect(statSync(cli).mode & 0o111).not.toBe(0);
    expect(readlinkSync(join(home, ".local/bin/yunagi"))).toBe(cli);

    const desktop = join(home, ".local/share/applications/io.github.misty-swan.yunagi.desktop");
    const entry = readFileSync(desktop, "utf8");
    expect(entry).toContain("[Desktop Entry]");
    expect(entry).toContain(`Exec="${app}"`);

    for (const placed of [app, cli, join(home, ".local/bin/yunagi"), desktop]) {
      expect(r.out).toContain(placed);
    }
    // 一時ファイルを残さない
    expect(existsSync(join(home, ".local/opt/yunagi/.Yunagi.AppImage.new"))).toBe(false);
    expect(existsSync(join(home, ".local/opt/yunagi/bin/.yunagi.new"))).toBe(false);
  });

  it("XDG_DATA_HOME があればデスクトップエントリをそこに置く", () => {
    stockRelease("x86_64");
    const data = join(root, "xdg-data");
    const r = run({ XDG_DATA_HOME: data });
    expect(r.status).toBe(0);
    expect(existsSync(join(data, "applications/io.github.misty-swan.yunagi.desktop"))).toBe(true);
  });

  it("aarch64 は arm64 の配布物を取る", () => {
    stockRelease("arm64");
    const r = run({}, { s: "Linux", m: "aarch64" });
    expect(r.err).toBe("");
    expect(r.status).toBe(0);
    expect(r.out).toContain("(linux, arm64)");
  });

  it("もう入っているときは置き換える（アプリの上書きは名前の付け替えで行う）", () => {
    stockRelease("x86_64");
    expect(run().status).toBe(0);
    put(`Yunagi_${VERSION}_x86_64.AppImage`, "newer-bytes");
    expect(run().status).toBe(0);
    expect(readFileSync(join(home, ".local/opt/yunagi/Yunagi.AppImage"), "utf8")).toBe("newer-bytes");
  });

  it("~/.local/bin/yunagi がリンクでない普通のファイルなら、上書きせずに止まる", () => {
    stockRelease("x86_64");
    mkdirSync(join(home, ".local/bin"), { recursive: true });
    writeFileSync(join(home, ".local/bin/yunagi"), "mine");
    const r = run();
    expect(r.status).toBe(1);
    expect(r.err).toContain("not a link");
    expect(readFileSync(join(home, ".local/bin/yunagi"), "utf8")).toBe("mine");
  });

  it("前のリンクは付け替える", () => {
    stockRelease("x86_64");
    mkdirSync(join(home, ".local/bin"), { recursive: true });
    symlinkSync("/nonexistent", join(home, ".local/bin/yunagi"));
    expect(run().status).toBe(0);
    expect(lstatSync(join(home, ".local/bin/yunagi")).isSymbolicLink()).toBe(true);
    expect(readlinkSync(join(home, ".local/bin/yunagi"))).toBe(join(home, ".local/opt/yunagi/bin/yunagi"));
  });

  it("ハッシュが合わなければ中止し、何も置かない", () => {
    put(`Yunagi_${VERSION}_x86_64.AppImage`, "appimage-bytes", "0".repeat(64));
    put(`yunagi-cli_${VERSION}_x86_64.tar.gz`, tarGz({ yunagi: "x" }));
    const r = run();
    expect(r.status).toBe(1);
    expect(r.err).toContain("SHA-256 mismatch");
    expect(existsSync(join(home, ".local"))).toBe(false);
  });

  it(".sha256 がハッシュの形でなければ中止する", () => {
    put(`Yunagi_${VERSION}_x86_64.AppImage`, "appimage-bytes", "not-a-hash");
    const r = run();
    expect(r.status).toBe(1);
    expect(r.err).toContain("does not hold a SHA-256 hash");
    expect(existsSync(join(home, ".local"))).toBe(false);
  });

  it("配布物が無ければ、取れなかった URL を示して中止する", () => {
    const r = run();
    expect(r.status).toBe(1);
    expect(r.err).toContain("could not download");
    expect(r.err).toContain(`v${VERSION}/Yunagi_${VERSION}_x86_64.AppImage`);
  });

  it("CLI の配布物に yunagi が入っていなければ中止する", () => {
    put(`Yunagi_${VERSION}_x86_64.AppImage`, "appimage-bytes");
    put(`yunagi-cli_${VERSION}_x86_64.tar.gz`, tarGz({ other: "x" }));
    const r = run();
    expect(r.status).toBe(1);
    expect(existsSync(join(home, ".local/opt"))).toBe(false);
  });

  it("YUNAGI_BASE_URL の末尾に / があっても動く", () => {
    stockRelease("x86_64");
    const r = run({ YUNAGI_BASE_URL: `file://${dist}/` });
    expect(r.err).toBe("");
    expect(r.status).toBe(0);
  });

  it("コピーに失敗したときは、前の AppImage を残し、一時ファイルも残さない", () => {
    stockRelease("x86_64");
    expect(run().status).toBe(0);
    // AppImage の一時ファイルの場所をフォルダにして、cp を失敗させる
    mkdirSync(join(home, ".local/opt/yunagi/.Yunagi.AppImage.new/x"), { recursive: true });
    put(`Yunagi_${VERSION}_x86_64.AppImage`, "newer-bytes");
    const r = run();
    expect(r.status).not.toBe(0);
    expect(readFileSync(join(home, ".local/opt/yunagi/Yunagi.AppImage"), "utf8")).toBe("appimage-bytes");
    expect(existsSync(join(home, ".local/opt/yunagi/bin/.yunagi.new"))).toBe(false);
  });

  it("CLI の配布物のメンバー名が ./yunagi でも読める", () => {
    put(`Yunagi_${VERSION}_x86_64.AppImage`, "appimage-bytes");
    const src = mkdtempSync(join(root, "dot-"));
    writeFileSync(join(src, "yunagi"), "#!/bin/sh\n", { mode: 0o755 });
    const out = join(root, "dot.tar.gz");
    expect(spawnSync("tar", ["-czf", out, "-C", src, "."]).status).toBe(0);
    put(`yunagi-cli_${VERSION}_x86_64.tar.gz`, readFileSync(out));
    const r = run();
    expect(r.err).toBe("");
    expect(r.status).toBe(0);
  });

  it("置き場所に Exec で特別に扱われる文字（%）があるときは、デスクトップエントリを作らずに知らせる", () => {
    stockRelease("x86_64");
    const odd = join(root, "home%odd");
    mkdirSync(odd);
    const r = run({ HOME: odd });
    expect(r.status).toBe(0);
    expect(r.out).toContain("No desktop entry was made");
    expect(existsSync(join(odd, ".local/share/applications"))).toBe(false);
    expect(existsSync(join(odd, ".local/opt/yunagi/Yunagi.AppImage"))).toBe(true);
  });

  it("curl | sh と同じく、標準入力から読ませても動く", () => {
    stockRelease("x86_64");
    const r = run({}, { s: "Linux", m: "x86_64" }, true);
    expect(r.err).toBe("");
    expect(r.status).toBe(0);
  });
});

describe("install.sh：判定と版", () => {
  it("未対応の OS は、対応する OS を示して終わる", () => {
    const r = run({}, { s: "FreeBSD", m: "x86_64" });
    expect(r.status).toBe(1);
    expect(r.err).toContain("unsupported OS: FreeBSD");
  });

  it("未対応のアーキテクチャは、対応するものを示して終わる", () => {
    const r = run({}, { s: "Linux", m: "riscv64" });
    expect(r.status).toBe(1);
    expect(r.err).toContain("unsupported architecture: riscv64");
  });

  it("YUNAGI_VERSION は v 付きでも受ける", () => {
    stockRelease("x86_64");
    const r = run({ YUNAGI_VERSION: `v${VERSION}` });
    expect(r.status).toBe(0);
    expect(r.out).toContain(`Installing Yunagi ${VERSION}`);
  });

  it("形のおかしい版は、取りに行く前に断る", () => {
    for (const bad of ["../x", "1.2/3", "abc", "1.2 3"]) {
      const r = run({ YUNAGI_VERSION: bad });
      expect(r.status, bad).toBe(1);
      expect(r.err, bad).toContain("invalid version");
    }
  });

  it("版を指定しなければ latest.json の version を使う", () => {
    stockRelease("x86_64");
    const latest = join(root, "latest.json");
    writeFileSync(latest, JSON.stringify({ version: VERSION, notes: "", platforms: {} }, null, 2));
    const r = run({ YUNAGI_VERSION: "", YUNAGI_LATEST_URL: `file://${latest}` });
    expect(r.err).toBe("");
    expect(r.status).toBe(0);
    expect(r.out).toContain(`Installing Yunagi ${VERSION}`);
  });

  it("latest.json に version が無ければ中止する", () => {
    const latest = join(root, "latest.json");
    writeFileSync(latest, "{}");
    const r = run({ YUNAGI_VERSION: "", YUNAGI_LATEST_URL: `file://${latest}` });
    expect(r.status).toBe(1);
    expect(r.err).toContain("could not read the version");
  });

  it("配布先がまだ仮の表記のままなら、設定する変数を示して中止する", () => {
    const r = run({ YUNAGI_BASE_URL: "" });
    expect(r.status).toBe(1);
    expect(r.err).toContain("YUNAGI_BASE_URL");
    expect(existsSync(join(home, ".local"))).toBe(false);
  });
});

describe("install.sh：macOS", () => {
  it("~/Applications/Yunagi.app に展開し、アプリの中の yunagi へリンクする", () => {
    stockRelease("arm64");
    const r = run({}, { s: "Darwin", m: "arm64" });
    expect(r.err).toBe("");
    expect(r.status).toBe(0);
    const cli = join(home, "Applications/Yunagi.app/Contents/MacOS/yunagi");
    expect(existsSync(cli)).toBe(true);
    expect(readlinkSync(join(home, ".local/bin/yunagi"))).toBe(cli);
    expect(r.out).toContain(join(home, "Applications/Yunagi.app"));
    // macOS には Linux のデスクトップエントリも Wayland の案内も出さない
    expect(existsSync(join(home, ".local/share"))).toBe(false);
    expect(r.out).not.toContain("Wayland");
  });

  it("入っている Yunagi.app は置き換える", () => {
    stockRelease("arm64");
    mkdirSync(join(home, "Applications/Yunagi.app"), { recursive: true });
    writeFileSync(join(home, "Applications/Yunagi.app/old"), "old");
    expect(run({}, { s: "Darwin", m: "arm64" }).status).toBe(0);
    expect(existsSync(join(home, "Applications/Yunagi.app/old"))).toBe(false);
  });

  it("Rosetta の下のシェル（uname が x86_64）でも、arm64 の Mac なら arm64 の配布物を取る", () => {
    stockRelease("arm64");
    writeFileSync(join(bin, "sysctl"), '#!/bin/sh\necho 1\n', { mode: 0o755 });
    const r = run({}, { s: "Darwin", m: "x86_64" });
    expect(r.err).toBe("");
    expect(r.status).toBe(0);
    expect(r.out).toContain("(macos, arm64)");
  });

  it("展開に失敗したときは、前の Yunagi.app を残す", () => {
    stockRelease("arm64");
    mkdirSync(join(home, "Applications/Yunagi.app"), { recursive: true });
    writeFileSync(join(home, "Applications/Yunagi.app/old"), "old");
    // 展開の結果に Contents/MacOS/yunagi が無い配布物
    put(`Yunagi_${VERSION}_arm64.app.tar.gz`, tarGz({ "Yunagi.app/other": "x" }));
    const r = run({}, { s: "Darwin", m: "arm64" });
    expect(r.status).toBe(1);
    expect(readFileSync(join(home, "Applications/Yunagi.app/old"), "utf8")).toBe("old");
  });

  it("x86_64 の Mac は x86_64 の配布物を取る", () => {
    stockRelease("x86_64");
    const r = run({}, { s: "Darwin", m: "x86_64" });
    expect(r.status).toBe(0);
    expect(r.out).toContain("(macos, x86_64)");
  });
});

describe("install.sh：PATH の案内", () => {
  it("~/.local/bin が PATH に無ければ、足す方法を案内する", () => {
    stockRelease("x86_64");
    const r = run();
    expect(r.out).toContain("is not in your PATH");
    expect(r.out).toContain('export PATH="$HOME/.local/bin:$PATH"');
  });

  it("PATH に末尾が / の形で入っていても、入っているとみなす", () => {
    stockRelease("x86_64");
    const r = run({ PATH: `${home}/.local/bin/:${bin}:${process.env.PATH}` });
    expect(r.out).not.toContain("is not in your PATH");
  });

  it("PATH に入っていれば案内しない", () => {
    stockRelease("x86_64");
    const r = run({ PATH: `${home}/.local/bin:${bin}:${process.env.PATH}` });
    expect(r.out).not.toContain("is not in your PATH");
  });
});

describe("install.sh：Wayland の案内（Linux だけ）", () => {
  const wayland = (env: Record<string, string>) => {
    stockRelease("x86_64");
    return run(env);
  };

  it("XDG_SESSION_TYPE=wayland なら出す", () => {
    expect(wayland({ XDG_SESSION_TYPE: "wayland" }).out).toContain("On Wayland");
  });

  it("XDG_SESSION_TYPE が無く WAYLAND_DISPLAY があれば出す（TTY から起動した Hyprland・Sway など）", () => {
    expect(wayland({ WAYLAND_DISPLAY: "wayland-1" }).out).toContain("On Wayland");
    expect(wayland({ XDG_SESSION_TYPE: "tty", WAYLAND_DISPLAY: "wayland-1" }).out).toContain("On Wayland");
  });

  it("x11 と明示されていれば、WAYLAND_DISPLAY が残っていても出さない", () => {
    expect(wayland({ XDG_SESSION_TYPE: "x11", WAYLAND_DISPLAY: "wayland-1" }).out).not.toContain("Wayland");
  });

  it("どちらも無ければ出さない", () => {
    expect(wayland({}).out).not.toContain("Wayland");
  });

  it("コマンド名・--note の別のキー・フルパスの書き方を含み、設定の Wayland の文と食い違わない", () => {
    const out = wayland({ XDG_SESSION_TYPE: "wayland" }).out;
    // PATH に ~/.local/bin が無いので、フルパス（展開した $HOME の実際のパス）で案内する
    expect(out).toContain(`${home}/.local/bin/yunagi show`);
    expect(out).toContain(`'${home}/.local/bin/yunagi show --note'`);
    expect(out).toContain("full path");
    for (const place of ["GNOME", "KDE", "Hyprland, Sway"]) expect(out).toContain(place);

    // 設定の文（英語）：src/lib/i18n_settings.ts の operation.global.wayland
    const settings = readFileSync(resolve(__dirname, "../src/lib/i18n_settings.ts"), "utf8");
    expect(settings).toContain("assign the command yunagi show to a key of your choice");
    expect(settings).toContain("Wayland では、このショートカットが効かないことがあります");
    expect(out).toContain("On Wayland the shortcut Mod+Shift+D may not work.");
    expect(out).toMatch(/assign the command\n {2}.*yunagi show\nto a key of your choice in your desktop environment's keyboard settings/);
  });

  it("PATH に ~/.local/bin があれば、コマンドは yunagi show と書く", () => {
    stockRelease("x86_64");
    const out = run({ XDG_SESSION_TYPE: "wayland", PATH: `${home}/.local/bin:${bin}:${process.env.PATH}` }).out;
    expect(out).toContain("assign the command\n  yunagi show\n");
    expect(out).toContain("'yunagi show --note'");
  });
});

// install.ps1：この環境に pwsh（PowerShell 7）があるときだけ走る（GitHub の ubuntu-24.04 には入っている）。
const pwsh = spawnSync("pwsh", ["-NoProfile", "-Command", "$PSVersionTable.PSVersion.Major"], { encoding: "utf8" });
const hasPwsh = pwsh.status === 0;

describe.skipIf(!hasPwsh)("install.ps1（pwsh がある環境だけ）", () => {
  function ps(script: string) {
    const r = spawnSync("pwsh", ["-NoProfile", "-NonInteractive", "-Command", script], {
      encoding: "utf8",
      env: { ...process.env, YUNAGI_INSTALL_NO_RUN: "1" },
    });
    return { status: r.status, out: r.stdout.trim(), err: r.stderr };
  }
  const load = `. '${PS1}'; `;

  it("構文が正しい", () => {
    const r = ps(
      `$e = $null; [System.Management.Automation.Language.Parser]::ParseFile('${PS1}', [ref]$null, [ref]$e) | Out-Null; $e.Count`,
    );
    expect(r.out).toBe("0");
  });

  it("ASCII だけで書いてある（Windows PowerShell 5.1 は BOM の無いファイルを ANSI で読む）", () => {
    for (const byte of readFileSync(PS1)) expect(byte).toBeLessThan(0x80);
    for (const byte of readFileSync(SCRIPT)) expect(byte).toBeLessThan(0x80);
  });

  it("読み込むだけでは何も入れない", () => {
    const r = ps(`${load} 'loaded'`);
    expect(r.out).toBe("loaded");
  });

  it("アーキテクチャの名前を配布物の名前に読み替える", () => {
    expect(ps(`${load} Get-YunagiArch 'X64'`).out).toBe("x86_64");
    expect(ps(`${load} Get-YunagiArch 'Arm64'`).out).toBe("arm64");
    const bad = ps(`${load} Get-YunagiArch 'X86'`);
    expect(bad.status).not.toBe(0);
    expect(bad.err).toContain("unsupported architecture");
  });

  it("版は v 付きでも受け、形のおかしいものは断る", () => {
    expect(ps(`${load} ConvertTo-YunagiVersion 'v1.2.3'`).out).toBe("1.2.3");
    expect(ps(`${load} ConvertTo-YunagiVersion '1.2.3'`).out).toBe("1.2.3");
    expect(ps(`${load} ConvertTo-YunagiVersion '..\\x'`).status).not.toBe(0);
  });

  it(".sha256 の読み方：ハッシュだけでも「ハッシュ  ファイル名」でも読め、形が違えば断る", () => {
    const h = sha256("x");
    expect(ps(`${load} Read-Sha256File '${h}'`).out).toBe(h);
    expect(ps(`${load} Read-Sha256File '${h.toUpperCase()}  a.exe'`).out).toBe(h);
    expect(ps(`${load} Read-Sha256File 'nothash'`).status).not.toBe(0);
  });

  it("ハッシュの照合：合えば通り、違えば止まる", () => {
    const file = join(root, "a.bin");
    writeFileSync(file, "x");
    expect(ps(`${load} Assert-Sha256 '${file}' '${sha256("x")}'; 'ok'`).out).toBe("ok");
    const bad = ps(`${load} Assert-Sha256 '${file}' '${"0".repeat(64)}'`);
    expect(bad.status).not.toBe(0);
    expect(bad.err).toContain("SHA-256 mismatch");
  });

  it("PATH への追加：無ければ足し、あれば（大文字小文字・末尾の \\ の違いも）足さない", () => {
    expect(ps(`${load} Get-PathWithEntry 'C:\\a;C:\\b' 'C:\\x\\bin'`).out).toBe("C:\\a;C:\\b;C:\\x\\bin");
    expect(ps(`${load} Get-PathWithEntry '' 'C:\\x\\bin'`).out).toBe("C:\\x\\bin");
    expect(ps(`${load} $null -eq (Get-PathWithEntry 'C:\\a;c:\\X\\BIN\\' 'C:\\x\\bin')`).out).toBe("True");
    // 空の項目だけの PATH に足しても、先頭に空の項目を作らない
    expect(ps(`${load} Get-PathWithEntry ';; ;' 'C:\\x\\bin'`).out).toBe("C:\\x\\bin");
    // 展開前の %VAR% の項目が同じ場所を指しているときも、足さない
    expect(
      ps(`$env:YT_BASE='C:\\x'; ${load} $null -eq (Get-PathWithEntry 'C:\\a;%YT_BASE%\\bin' 'C:\\x\\bin')`).out,
    ).toBe("True");
  });
});
