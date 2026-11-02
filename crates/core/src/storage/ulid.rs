//! 主キーに使う ULID（docs/design/02-data-model.md 1章）。
//!
//! ULID は 128 ビットの識別子で、先頭 48 ビットが UNIX 時刻（ミリ秒）、残り 80 ビットが乱数。
//! 文字列（Crockford の Base32 で 26 文字）にしても時刻の順に並ぶので、端末間で衝突しにくく、
//! 作った順に並べられる（<https://github.com/ulid/spec>）。
//!
//! このプロセスの中で作る ULID は、必ず前に作ったものより大きくする（同じミリ秒の中では
//! 乱数の部分を 1 ずつ増やす。時計が戻ったときも前の時刻を使い続ける）。

use std::fmt;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::ToSql;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};

/// Crockford の Base32 の文字（I・L・O・U を除く）。
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 乱数の部分（80 ビット）の最大値。
const RANDOM_MAX: u128 = (1 << 80) - 1;

/// 時刻の部分（48 ビット）の最大値。
const TIME_MAX: u64 = (1 << 48) - 1;

/// ULID。データベースには 26 文字の文字列（大文字）で保存する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ulid(u128);

/// 最後に作った ULID。次の ULID をこれより大きくするために使う。
static LAST: Mutex<u128> = Mutex::new(0);

impl Ulid {
    /// いまの時刻で新しい ULID を作る。
    ///
    /// # Panics
    ///
    /// OS から乱数を得られなかったとき（ふつうは起きない）。
    pub fn new() -> Self {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let now_ms = u64::try_from(now_ms).unwrap_or(u64::MAX).min(TIME_MAX);
        let mut random = [0u8; 16];
        getrandom::fill(&mut random[6..]).expect("failed to get random bytes from the OS");
        let random = u128::from_be_bytes(random);

        // 毒された（ほかのスレッドが持ったまま止まった）ロックでも、値は壊れていないので使う
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        let next = next_after(*last, now_ms, random);
        *last = next;
        Self(next)
    }

    /// 数値から作る。固定の ID（`cli` の agent_client など）やテストに使う。
    pub const fn from_u128(value: u128) -> Self {
        Self(value)
    }

    /// 数値としての値。
    pub const fn to_u128(self) -> u128 {
        self.0
    }

    /// 時刻の部分（UNIX 時刻、ミリ秒）。
    pub const fn timestamp_ms(self) -> u64 {
        (self.0 >> 80) as u64
    }
}

impl Default for Ulid {
    fn default() -> Self {
        Self::new()
    }
}

/// `last` より大きい ULID を、時刻 `now_ms` と乱数 `random`（下位 80 ビットを使う）から作る。
///
/// - `now_ms` が `last` の時刻より後なら、そのまま `now_ms` と `random` で作る。
/// - 同じか前（同じミリ秒の中、または時計が戻った）なら、`last` に 1 を足す。乱数の部分が
///   あふれたら時刻の部分に繰り上がる（1 ミリ秒先の時刻になるが、順序は保たれる）。
fn next_after(last: u128, now_ms: u64, random: u128) -> u128 {
    let candidate = (u128::from(now_ms) << 80) | (random & RANDOM_MAX);
    if (last >> 80) < u128::from(now_ms) {
        candidate
    } else {
        // 2^128 個を作り切ることは無いので、あふれは考えない
        last + 1
    }
}

impl fmt::Display for Ulid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = [0u8; 26];
        let mut value = self.0;
        for slot in buf.iter_mut().rev() {
            *slot = ALPHABET[(value & 0x1f) as usize];
            value >>= 5;
        }
        // ALPHABET は ASCII だけなので、必ず UTF-8 として正しい
        f.write_str(std::str::from_utf8(&buf).expect("ULID characters are ASCII"))
    }
}

/// ULID の文字列として正しくない。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseUlidError;

impl fmt::Display for ParseUlidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid ULID (expected 26 Crockford Base32 characters)")
    }
}

impl std::error::Error for ParseUlidError {}

impl FromStr for Ulid {
    type Err = ParseUlidError;

    /// 26 文字の Crockford の Base32 を読む。小文字も受け付ける。
    /// 128 ビットに収まらない値（先頭の文字が 8 以上）は誤りとする。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        if bytes.len() != 26 {
            return Err(ParseUlidError);
        }
        let mut value: u128 = 0;
        for (i, &b) in bytes.iter().enumerate() {
            let digit = decode_char(b).ok_or(ParseUlidError)?;
            // 26 文字 × 5 ビット = 130 ビットなので、先頭の文字は 3 ビット（0〜7）まで
            if i == 0 && digit > 7 {
                return Err(ParseUlidError);
            }
            value = (value << 5) | u128::from(digit);
        }
        Ok(Self(value))
    }
}

/// 1 文字を 0〜31 の値にする。Base32 の文字でなければ `None`。
fn decode_char(b: u8) -> Option<u8> {
    let upper = b.to_ascii_uppercase();
    ALPHABET.iter().position(|&c| c == upper).map(|i| i as u8)
}

impl ToSql for Ulid {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.to_string()))
    }
}

impl FromSql for Ulid {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        value
            .as_str()?
            .parse()
            .map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_26_characters_and_round_trips() {
        let id = Ulid::new();
        let s = id.to_string();
        assert_eq!(s.len(), 26);
        assert!(s.bytes().all(|b| ALPHABET.contains(&b)));
        assert_eq!(s.parse::<Ulid>(), Ok(id));
    }

    #[test]
    fn encodes_known_values() {
        assert_eq!(Ulid::from_u128(0).to_string(), "00000000000000000000000000");
        assert_eq!(Ulid::from_u128(1).to_string(), "00000000000000000000000001");
        assert_eq!(
            Ulid::from_u128(u128::MAX).to_string(),
            "7ZZZZZZZZZZZZZZZZZZZZZZZZZ"
        );
    }

    #[test]
    fn parses_lowercase() {
        let id = Ulid::new();
        assert_eq!(id.to_string().to_lowercase().parse::<Ulid>(), Ok(id));
    }

    #[test]
    fn rejects_invalid_strings() {
        for s in [
            "",
            "0000000000000000000000000",   // 25 文字
            "000000000000000000000000000", // 27 文字
            "8ZZZZZZZZZZZZZZZZZZZZZZZZZ",  // 128 ビットを超える
            "0000000000000000000000000U",  // Base32 に無い文字
            "0000000000000000000000000-",
            "000000000000000000000000０", // 全角（26 バイトではない）
        ] {
            assert_eq!(s.parse::<Ulid>(), Err(ParseUlidError), "{s:?}");
        }
    }

    #[test]
    fn carries_the_current_time() {
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let id = Ulid::new();
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        // ほかのテストが同時に作った ULID より大きくするため、時刻が先へ進むことがある
        assert!(
            id.timestamp_ms() >= before,
            "{} < {before}",
            id.timestamp_ms()
        );
        assert!(id.timestamp_ms() <= after + 1);
    }

    #[test]
    fn increases_strictly_even_within_the_same_millisecond() {
        let ids: Vec<Ulid> = (0..10_000).map(|_| Ulid::new()).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
        // 文字列でも同じ順に並ぶ
        let strings: Vec<String> = ids.iter().map(Ulid::to_string).collect();
        assert!(strings.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn next_after_uses_the_new_time_when_it_advances() {
        let last = (5u128 << 80) | 123;
        assert_eq!(next_after(last, 6, 7), (6u128 << 80) | 7);
    }

    #[test]
    fn next_after_increments_within_the_same_millisecond_or_when_the_clock_goes_back() {
        let last = (5u128 << 80) | 123;
        assert_eq!(next_after(last, 5, 0), last + 1);
        assert_eq!(next_after(last, 4, u128::MAX), last + 1);
    }

    #[test]
    fn next_after_carries_into_the_time_part_when_the_random_part_overflows() {
        let last = (5u128 << 80) | RANDOM_MAX;
        assert_eq!(next_after(last, 5, 0), 6u128 << 80);
    }
}
