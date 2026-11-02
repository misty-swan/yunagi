//! ドメイン層：日付ボード、TODO、障害、振り返り、ノート、持ち越し
//! （docs/design/01-architecture.md 2章）。中身は後続の課題で作る。

#[cfg(test)]
mod tests {
    #[test]
    fn module_is_linked() {
        assert_eq!(module_path!(), "yunagi_core::domain::tests");
    }
}
