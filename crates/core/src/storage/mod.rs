//! ストレージ：SQLite（WAL）＋ FTS5 全文検索
//! （docs/design/01-architecture.md 2章・3章）。中身は後続の課題で作る。

#[cfg(test)]
mod tests {
    #[test]
    fn module_is_linked() {
        assert_eq!(module_path!(), "yunagi_core::storage::tests");
    }
}
