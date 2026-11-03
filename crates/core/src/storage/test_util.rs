//! storage のテストで共通に使う補助（テストのビルドにだけある）。

use std::fs;
use std::path::Path;

/// `dir` の中の名前の一覧（並べたもの）。`dir` が無ければ空。
pub(super) fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|es| {
            es.map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// データベースのファイル `path` の2ページ目以降の中身を壊す（見出しは残すので、SQLite の
/// ファイルとしては開ける。読むか整合性を確かめると、壊れていることに気づく）。
pub(super) fn corrupt_pages(path: &Path) {
    let mut bytes = fs::read(path).unwrap();
    let page_size = usize::from(u16::from_be_bytes([bytes[16], bytes[17]]));
    for b in &mut bytes[page_size + 8..] {
        *b = 0xA5;
    }
    fs::write(path, &bytes).unwrap();
}
