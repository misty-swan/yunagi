//! 複数の表への書き込みを1つのトランザクションにまとめる補助。
//!
//! ドメイン層の書き込み（ノート・項目の操作）は、本文や項目の行・day の行・change_log の記録を
//! 一緒に書く。どれかが失敗したら全部を取り消すため、[`in_transaction`] の中で書く。

use rusqlite::Connection;

/// `f` を1つのトランザクションの中で行い、成功したら確定し、失敗したら取り消す。
/// 呼び出し側のトランザクションの中で呼べば、セーブポイントを使ってその一部になる（`BEGIN` は
/// 入れ子にできないため）。確定は呼び出し側が行う。
///
/// - `f` の失敗は、取り消しにも失敗したときでも `f` の失敗のまま返す（行き違いの最新の本文などを
///   失わないため）。
/// - 確定（`COMMIT`）の失敗と、`f` の途中の panic でも取り消す。トランザクションが開いたまま
///   残ると、以後の書き込みがすべてその中に入り、確定されないため。
pub(crate) fn in_transaction<T, E: From<rusqlite::Error>>(
    conn: &Connection,
    f: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let guard = TransactionGuard::begin(conn)?;
    let value = f()?; // 失敗なら guard の drop で取り消す
    guard.commit()?;
    Ok(value)
}

/// [`in_transaction`] のトランザクション。確定せずに手放すと（失敗・panic）取り消す。
struct TransactionGuard<'c> {
    conn: &'c Connection,
    /// 外側のトランザクションか（`false` なら呼び出し側のトランザクションの中のセーブポイント）。
    outermost: bool,
    done: bool,
}

impl<'c> TransactionGuard<'c> {
    /// セーブポイントの名前。入れ子にしても、SQLite は同じ名前のうち最も内側のものを外す・戻すので、
    /// 1つの名前でよい。
    const SAVEPOINT: &'static str = "yunagi_tx";

    fn begin(conn: &'c Connection) -> rusqlite::Result<Self> {
        let outermost = conn.is_autocommit();
        if outermost {
            conn.execute_batch("BEGIN")?;
        } else {
            conn.execute_batch(&format!("SAVEPOINT {}", Self::SAVEPOINT))?;
        }
        Ok(Self {
            conn,
            outermost,
            done: false,
        })
    }

    /// 確定する。失敗したら、drop で取り消す。
    fn commit(mut self) -> rusqlite::Result<()> {
        if self.outermost {
            self.conn.execute_batch("COMMIT")?;
        } else {
            self.conn
                .execute_batch(&format!("RELEASE {}", Self::SAVEPOINT))?;
        }
        self.done = true;
        Ok(())
    }
}

impl Drop for TransactionGuard<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // 取り消しの失敗は返す先が無い（元の失敗を返すほうを優先する）ので捨てる
        let _ = if self.outermost {
            self.conn.execute_batch("ROLLBACK")
        } else {
            // ROLLBACK TO はセーブポイントを残すので、続けて外す
            self.conn.execute_batch(&format!(
                "ROLLBACK TO {name}; RELEASE {name}",
                name = Self::SAVEPOINT
            ))
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageError;
    use crate::storage::table::day;
    use crate::storage::table::test_support::{at, date, open};

    type Result = std::result::Result<(), StorageError>;

    fn ensure_day(conn: &Connection, d: &str) -> Result {
        day::ensure(conn, date(d), "Asia/Tokyo", at("2026-11-03T00:00:00Z")).map(|_| ())
    }

    fn fail() -> Result {
        Err(rusqlite::Error::InvalidQuery.into())
    }

    #[test]
    fn commits_on_success_and_rolls_back_on_failure() {
        let (db, _dir) = open();
        let conn = db.connection();
        in_transaction(conn, || ensure_day(conn, "2026-11-03")).unwrap();
        let failed = in_transaction(conn, || {
            ensure_day(conn, "2026-11-04")?;
            fail()
        });
        assert!(failed.is_err());
        assert!(conn.is_autocommit());
        assert!(day::get(conn, date("2026-11-03")).unwrap().is_some());
        assert_eq!(day::get(conn, date("2026-11-04")).unwrap(), None);
    }

    #[test]
    fn a_panic_inside_the_transaction_rolls_it_back() {
        let (db, _dir) = open();
        let conn = db.connection();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result = in_transaction(conn, || {
                ensure_day(conn, "2026-11-03")?;
                panic!("boom");
            });
        }));
        assert!(result.is_err());
        assert!(conn.is_autocommit());
        assert_eq!(day::get(conn, date("2026-11-03")).unwrap(), None);
        // 以後の書き込みは確定される
        in_transaction(conn, || ensure_day(conn, "2026-11-04")).unwrap();
        assert!(conn.is_autocommit());
        assert!(day::get(conn, date("2026-11-04")).unwrap().is_some());
    }

    #[test]
    fn nested_calls_join_the_outer_transaction() {
        let (db, _dir) = open();
        let conn = db.connection();
        // 内側の失敗は内側の分だけを取り消す
        in_transaction(conn, || {
            ensure_day(conn, "2026-11-03")?;
            let inner = in_transaction(conn, || {
                ensure_day(conn, "2026-11-04")?;
                fail()
            });
            assert!(inner.is_err());
            assert!(!conn.is_autocommit());
            in_transaction(conn, || ensure_day(conn, "2026-11-05"))
        })
        .unwrap();
        assert!(day::get(conn, date("2026-11-03")).unwrap().is_some());
        assert_eq!(day::get(conn, date("2026-11-04")).unwrap(), None);
        assert!(day::get(conn, date("2026-11-05")).unwrap().is_some());

        // 呼び出し側が確定せずに手放すと、内側で確定した分も取り消される
        {
            let tx = conn.unchecked_transaction().unwrap();
            in_transaction(&tx, || ensure_day(&tx, "2026-11-06")).unwrap();
        }
        assert_eq!(day::get(conn, date("2026-11-06")).unwrap(), None);
    }
}
