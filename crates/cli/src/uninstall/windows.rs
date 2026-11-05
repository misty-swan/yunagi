//! `yunagi uninstall` の Windows 向けの部分（ビルド・配布手順書 4.4）：ユーザーの `PATH` から `bin` を
//! 外す文字列の処理と、動いている `yunagi.exe` から切り離して起動する PowerShell の文を作る。
//!
//! 文字列を作る部分は OS に関係しない関数にして、Linux でも試験する（Windows の実機でしか確かめられない
//! のは、実際の起動とレジストリの読み書きだけ。テスト計画書 4.9）。

use std::path::{Path, PathBuf};

/// `yunagi.exe` から切り離して動かす後片付け。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detach {
    /// 終了を待つ `yunagi.exe` のプロセス ID。
    pub wait_pid: u32,
    /// NSIS のアンインストーラー（`<インストール先>\uninstall.exe`）。無ければ `None`。
    pub uninstaller: Option<PathBuf>,
    /// インストール先（`%LOCALAPPDATA%\Programs\Yunagi`）。アンインストーラーの後に、残りを消す。
    pub install_dir: PathBuf,
    /// 消す既定ディレクトリ（`--purge` のとき）。ロックファイルがあるため、終了を待ってから消す。
    pub default_dir: Option<PathBuf>,
}

/// 環境変数 `name` の値（`%NAME%` の展開に使う）。
pub type Vars<'a> = &'a dyn Fn(&str) -> Option<String>;

/// 文字列の中の `%NAME%` を展開する（`Environment.ExpandEnvironmentVariables` と同じく、
/// 未定義のものはそのまま残す）。
pub fn expand_env(text: &str, vars: Vars<'_>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match vars(name) {
                    Some(value) => out.push_str(&value),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 比べるための形：`%VAR%` を展開し、前後の引用符と末尾の `\`・`/` を除いて、小文字にする
/// （install.ps1 の `Get-PathWithEntry` と同じ同一視。ビルド・配布手順書 4.2）。
fn comparable(entry: &str, vars: Vars<'_>) -> String {
    let expanded = expand_env(entry.trim(), vars);
    expanded
        .trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

/// ユーザーの `PATH`（`%VAR%` を展開していない文字列）`current` から、`dir` と同じ場所の項目を外す。
/// 外すものが無ければ `None`。ほかの項目は、書かれたままの形で残す（`%JAVA_HOME%\bin` のような項目を
/// 展開した形で固定しないため）。
pub fn remove_entry(current: &str, dir: &str, vars: Vars<'_>) -> Option<String> {
    let target = comparable(dir, vars);
    let mut removed = false;
    let kept: Vec<&str> = current
        .split(';')
        .filter(|entry| {
            let same = !entry.trim().is_empty() && comparable(entry, vars) == target;
            removed |= same;
            !same
        })
        .collect();
    removed.then(|| kept.join(";"))
}

/// バイト列の Base64（`=` の詰め物つき）。
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(char::from(TABLE[(n >> 18) as usize & 63]));
        out.push(char::from(TABLE[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(TABLE[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(TABLE[n as usize & 63])
        } else {
            '='
        });
    }
    out
}

/// `powershell.exe -EncodedCommand` に渡す形（UTF-16LE を Base64 にしたもの）。引用符の扱いを
/// コマンドラインに持ち込まないため。
pub fn encoded_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&bytes)
}

/// PowerShell の単一引用符の文字列にする（中の `'` は `''`）。
fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "''"))
}

/// 切り離して動かす後片付けの PowerShell の文：`yunagi.exe` の終了を待ち、アンインストーラーを動かし
/// （`/S` は静かに、`_?=` はその場で動かして終わりを待つ。NSIS の決まりで最後の引数）、インストール先の
/// 残り（動き終えたアンインストーラー自身など）と、既定ディレクトリを消す。
pub fn detach_script(d: &Detach) -> String {
    let mut lines = vec![
        "$ErrorActionPreference = 'SilentlyContinue'".to_owned(),
        format!("Wait-Process -Id {}", d.wait_pid),
    ];
    if let Some(uninstaller) = &d.uninstaller {
        // 末尾の `\` は付けない（NSIS の `_?=` の値は、引用符なしで最後に置く）
        let in_place = format!(
            "'_?={}'",
            d.install_dir.to_string_lossy().replace('\'', "''")
        );
        lines.push(format!(
            "Start-Process -FilePath {} -ArgumentList '/S', {in_place} -Wait",
            quote(uninstaller),
        ));
    }
    lines.push(format!(
        "Remove-Item -LiteralPath {} -Recurse -Force",
        quote(&d.install_dir)
    ));
    if let Some(dir) = &d.default_dir {
        lines.push(format!(
            "Remove-Item -LiteralPath {} -Recurse -Force",
            quote(dir)
        ));
    }
    lines.join("\r\n")
}

/// ユーザーの `PATH` の生の値（`%VAR%` を展開していない文字列）を読む PowerShell の文。標準出力は UTF-8。
pub fn read_user_path_script() -> String {
    [
        "[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)",
        "$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment')",
        "if ($key) { [Console]::Out.Write([string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)); $key.Close() }",
    ]
    .join("\r\n")
}

/// ユーザーの `PATH` を `value` に書く PowerShell の文（`REG_EXPAND_SZ` のまま。実行中のプログラムに
/// 変更を伝えるため、使い捨てのユーザー変数で環境の変更の通知を送る。install.ps1 と同じ）。
pub fn write_user_path_script(value: &str) -> String {
    [
        format!(
            "$value = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{}'))",
            base64(value.as_bytes())
        ),
        "$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)"
            .to_owned(),
        "$key.SetValue('Path', $value, [Microsoft.Win32.RegistryValueKind]::ExpandString)"
            .to_owned(),
        "$key.Close()".to_owned(),
        "[Environment]::SetEnvironmentVariable('YUNAGI_ENV_REFRESH', '1', 'User')".to_owned(),
        "[Environment]::SetEnvironmentVariable('YUNAGI_ENV_REFRESH', $null, 'User')".to_owned(),
    ]
    .join("\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(name: &str) -> Option<String> {
        match name.to_ascii_uppercase().as_str() {
            "LOCALAPPDATA" => Some(r"C:\Users\a\AppData\Local".to_owned()),
            "JAVA_HOME" => Some(r"C:\jdk".to_owned()),
            _ => None,
        }
    }

    #[test]
    fn expands_percent_variables_and_keeps_unknown_ones() {
        assert_eq!(
            expand_env(r"%LOCALAPPDATA%\x", &vars),
            r"C:\Users\a\AppData\Local\x"
        );
        assert_eq!(expand_env(r"%NOPE%\x", &vars), r"%NOPE%\x");
        assert_eq!(expand_env("50%", &vars), "50%");
        assert_eq!(expand_env("a%%b", &vars), "a%%b");
    }

    #[test]
    fn removes_only_the_bin_entry_and_keeps_the_others_as_written() {
        let bin = r"C:\Users\a\AppData\Local\Programs\Yunagi\bin";
        let path = format!(r"%JAVA_HOME%\bin;{bin};C:\tools;;");
        assert_eq!(
            remove_entry(&path, bin, &vars).as_deref(),
            Some(r"%JAVA_HOME%\bin;C:\tools;;")
        );
        // 大文字小文字・末尾の `\`・`%VAR%` を展開した形・引用符は同じとみなす
        for same in [
            r"c:\users\A\appdata\local\programs\yunagi\BIN\",
            r"%LOCALAPPDATA%\Programs\Yunagi\bin",
            r#""C:\Users\a\AppData\Local\Programs\Yunagi\bin""#,
        ] {
            assert_eq!(
                remove_entry(&format!("C:\\x;{same}"), bin, &vars).as_deref(),
                Some(r"C:\x"),
                "{same}"
            );
        }
        // 無ければ書き換えない。似た名前の別の場所は外さない
        assert_eq!(remove_entry(r"C:\x;C:\tools", bin, &vars), None);
        assert_eq!(
            remove_entry(&format!("{bin}2;{bin}\\sub"), bin, &vars),
            None
        );
        assert_eq!(remove_entry("", bin, &vars), None);
        // 外した結果が空になっても、空の文字列を返す（項目が 1 つだけだったとき）
        assert_eq!(remove_entry(bin, bin, &vars).as_deref(), Some(""));
    }

    #[test]
    fn base64_matches_the_known_encodings() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // PowerShell の -EncodedCommand は UTF-16LE
        assert_eq!(encoded_command("A"), "QQA=");
    }

    #[test]
    fn detach_script_waits_then_uninstalls_then_removes_the_rest() {
        let script = detach_script(&Detach {
            wait_pid: 4242,
            uninstaller: Some(PathBuf::from(
                r"C:\Users\o'neil\AppData\Local\Programs\Yunagi\uninstall.exe",
            )),
            install_dir: PathBuf::from(r"C:\Users\o'neil\AppData\Local\Programs\Yunagi"),
            default_dir: Some(PathBuf::from(r"C:\Users\o'neil\AppData\Local\Yunagi")),
        });
        let lines: Vec<&str> = script.split("\r\n").collect();
        assert_eq!(lines[1], "Wait-Process -Id 4242");
        assert_eq!(
            lines[2],
            r"Start-Process -FilePath 'C:\Users\o''neil\AppData\Local\Programs\Yunagi\uninstall.exe' -ArgumentList '/S', '_?=C:\Users\o''neil\AppData\Local\Programs\Yunagi' -Wait"
        );
        assert_eq!(
            lines[3],
            r"Remove-Item -LiteralPath 'C:\Users\o''neil\AppData\Local\Programs\Yunagi' -Recurse -Force"
        );
        assert_eq!(
            lines[4],
            r"Remove-Item -LiteralPath 'C:\Users\o''neil\AppData\Local\Yunagi' -Recurse -Force"
        );
        // アンインストーラーが無い・データを消さないときは、その行を作らない
        let script = detach_script(&Detach {
            wait_pid: 1,
            uninstaller: None,
            install_dir: PathBuf::from(r"C:\i"),
            default_dir: None,
        });
        assert_eq!(script.split("\r\n").count(), 3);
        assert!(!script.contains("Start-Process"));
    }

    #[test]
    fn the_path_scripts_carry_the_value_without_quoting_problems() {
        let script = write_user_path_script("C:\\a'b;\"c\";%X%");
        assert!(script.contains("ExpandString"));
        assert!(!script.contains("a'b"), "the value travels as Base64");
        assert!(read_user_path_script().contains("DoNotExpandEnvironmentNames"));
    }
}
