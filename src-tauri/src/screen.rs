//! 画面の状態（アーキテクチャ設計書 4.1）：画面から知らされた入力中の欄・守る行と、画面の保存が
//! 失敗している間か。エージェントの書き込みの判定に、コアの [`UiState`] として渡す。

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};

use yunagi_core::domain::agent::{ItemField, ProtectedLines, UiState};
use yunagi_core::domain::logical_date::LogicalDate;
use yunagi_core::domain::text_doc::TextDoc;
use yunagi_core::storage::Ulid;

use crate::types::{CommandError, InvalidReason, UiReport};

/// 画面の状態。複数のスレッド（コマンドの処理・ローカル API）から読み書きする。
#[derive(Debug, Default)]
pub struct ScreenState {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// 画面からの保存の書き込みに失敗してから、次に成功するまでの間か。
    save_failing: bool,
    editing: HashSet<(Ulid, ItemField)>,
    editors: HashMap<(TextDoc, LogicalDate), ProtectedLines>,
}

impl ScreenState {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        crate::app_core::lock(&self.inner)
    }

    /// 画面から知らされた入力中の状態で置き換える。守る行の番号は昇順にし、重なりを除く。
    /// 番号が 0 の行があれば、何も変えずに [`InvalidReason::InvalidLine`]。
    pub fn report(&self, report: UiReport) -> Result<(), CommandError> {
        let mut editors = HashMap::new();
        for e in report.editors {
            if e.lines.contains(&0) {
                return Err(CommandError::InvalidArgument {
                    reason: InvalidReason::InvalidLine,
                });
            }
            let mut lines = e.lines;
            lines.sort_unstable();
            lines.dedup();
            if lines.is_empty() {
                continue;
            }
            editors.insert(
                (e.doc.into(), e.date.0),
                ProtectedLines { rev: e.rev, lines },
            );
        }
        let editing = report
            .editing_items
            .into_iter()
            .map(|e| (e.item_id.0, e.field.into()))
            .collect();
        let mut inner = self.lock();
        inner.editing = editing;
        inner.editors = editors;
        Ok(())
    }

    /// 画面からの保存の書き込みが失敗した（`true`）か、成功した（`false`）かを記録する。
    pub fn set_save_failing(&self, failing: bool) {
        self.lock().save_failing = failing;
    }
}

impl UiState for ScreenState {
    fn save_failing(&self) -> bool {
        self.lock().save_failing
    }

    fn is_editing(&self, item_id: Ulid, field: ItemField) -> bool {
        self.lock().editing.contains(&(item_id, field))
    }

    fn protected_lines(&self, doc: TextDoc, date: LogicalDate) -> Option<ProtectedLines> {
        self.lock().editors.get(&(doc, date)).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{self, EditingItem, EditorLines};

    fn date(s: &str) -> LogicalDate {
        s.parse().unwrap()
    }

    #[test]
    fn reports_replace_the_whole_state() {
        let screen = ScreenState::default();
        let a = Ulid::from_u128(1);
        let b = Ulid::from_u128(2);
        let d = date("2026-11-03");
        screen
            .report(UiReport {
                editing_items: vec![EditingItem {
                    item_id: types::Id(a),
                    field: types::ItemField::Text,
                }],
                editors: vec![
                    EditorLines {
                        doc: types::TextDoc::Note,
                        date: types::Date(d),
                        rev: 4,
                        lines: vec![3, 1, 3],
                    },
                    // 守る行の無いエディタは、知らせていないものとして扱う
                    EditorLines {
                        doc: types::TextDoc::Reflection,
                        date: types::Date(d),
                        rev: 1,
                        lines: vec![],
                    },
                ],
            })
            .unwrap();
        assert!(screen.is_editing(a, ItemField::Text));
        assert!(!screen.is_editing(a, ItemField::ResolutionNote));
        assert!(!screen.is_editing(b, ItemField::Text));
        assert_eq!(
            screen.protected_lines(TextDoc::Note, d),
            Some(ProtectedLines {
                rev: 4,
                lines: vec![1, 3]
            })
        );
        assert_eq!(screen.protected_lines(TextDoc::Reflection, d), None);
        assert_eq!(
            screen.protected_lines(TextDoc::Note, date("2026-11-04")),
            None
        );

        // 知らせ直すと、前の状態は残らない
        screen.report(UiReport::default()).unwrap();
        assert!(!screen.is_editing(a, ItemField::Text));
        assert_eq!(screen.protected_lines(TextDoc::Note, d), None);
    }

    #[test]
    fn line_zero_is_refused_without_changing_the_state() {
        let screen = ScreenState::default();
        let a = Ulid::from_u128(1);
        let editing = vec![EditingItem {
            item_id: types::Id(a),
            field: types::ItemField::ResolutionNote,
        }];
        screen
            .report(UiReport {
                editing_items: editing.clone(),
                editors: vec![],
            })
            .unwrap();
        let bad = UiReport {
            editing_items: vec![],
            editors: vec![EditorLines {
                doc: types::TextDoc::Note,
                date: types::Date(date("2026-11-03")),
                rev: 1,
                lines: vec![0, 2],
            }],
        };
        assert_eq!(
            screen.report(bad),
            Err(CommandError::InvalidArgument {
                reason: InvalidReason::InvalidLine
            })
        );
        assert!(screen.is_editing(a, ItemField::ResolutionNote));
    }

    #[test]
    fn save_failures_are_kept_until_cleared() {
        let screen = ScreenState::default();
        assert!(!screen.save_failing());
        screen.set_save_failing(true);
        assert!(screen.save_failing());
        // 入力中の状態を知らせ直しても、保存の失敗は画面からは変えられない
        screen.report(UiReport::default()).unwrap();
        assert!(screen.save_failing());
        screen.set_save_failing(false);
        assert!(!screen.save_failing());
    }
}
