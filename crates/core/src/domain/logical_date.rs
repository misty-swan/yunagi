//! 論理日付：「その日」を、切り替わり時刻（既定 4:00）を基準に求める
//! （docs/design/01-architecture.md 5章、docs/design/05-mcp-cli-spec.md 3章、
//! docs/design/02-data-model.md 3章 day）。
//!
//! 論理日付の算出はコアのこのモジュールだけが行い、GUI・CLI・MCP は同じ関数を使う。
//!
//! # 規則
//!
//! カレンダー日 `D` の論理日付は、`D` の切り替わり時刻の瞬間に始まり、
//! `D` の翌日の切り替わり時刻の瞬間の直前に終わる。ある瞬間の論理日付は、
//! その瞬間までに始まった論理日付のうち最も新しいものである。
//!
//! 切り替わり時刻を壁時計の時刻どうしで比べず、瞬間（UTC の時刻）で比べるのは、
//! 夏時間の切り替えで日付が逆戻りしないようにするため。
//!
//! - 切り替わり時刻が夏時間の開始で飛ばされる日（例：2:00〜3:00 が無い日に 2:30）は、
//!   飛ばされた後の最初の時刻（この例では 3:00）にその日が始まる。
//! - 切り替わり時刻が夏時間の終了で2回ある日（例：1:00〜2:00 が2回ある日に 1:30）は、
//!   1回目にその日が始まる。2回目の 1:00〜1:30 も、もう同じ日である。
//!
//! # 切り替わり時刻を変えたとき
//!
//! この規則は、新しく日付を求めるときにだけ使う。保存済みの日付（[`LogicalDate`]）は
//! 値として持つだけで、切り替わり時刻を変えても付け直さない（アーキテクチャ設計書 5章）。
//! そのため、このモジュールには保存済みの日付を切り替わり時刻から求め直す関数を置かない。

use std::fmt;
use std::str::FromStr;

use jiff::civil::{Date, Time};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{Timestamp, Zoned};

/// 日付の切り替わり時刻（設定 `day_boundary`。`HH:MM`、既定 `04:00`）。
///
/// 0:00〜23:59 の分単位で持つ。0:00 にすればカレンダー日付と同じになる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DayBoundary {
    hour: u8,
    minute: u8,
}

impl DayBoundary {
    /// 既定の切り替わり時刻（4:00）。
    pub const DEFAULT: Self = Self { hour: 4, minute: 0 };

    /// 時・分から作る。範囲外（24時以上・60分以上）なら `None`。
    pub fn new(hour: u8, minute: u8) -> Option<Self> {
        (hour < 24 && minute < 60).then_some(Self { hour, minute })
    }

    pub fn hour(self) -> u8 {
        self.hour
    }

    pub fn minute(self) -> u8 {
        self.minute
    }

    fn to_time(self) -> Time {
        // new() で範囲を確かめているので失敗しない
        Time::constant(self.hour as i8, self.minute as i8, 0, 0)
    }
}

impl Default for DayBoundary {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl fmt::Display for DayBoundary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

impl FromStr for DayBoundary {
    type Err = ParseError;

    /// `HH:MM`（2桁ずつ。例 `04:00`）だけを受け付ける。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseError::DayBoundary(s.to_owned());
        let b = s.as_bytes();
        if b.len() != 5 || b[2] != b':' {
            return Err(err());
        }
        let hour = two_digits(&b[0..2]).ok_or_else(err)?;
        let minute = two_digits(&b[3..5]).ok_or_else(err)?;
        Self::new(hour, minute).ok_or_else(err)
    }
}

/// 論理日付（`YYYY-MM-DD`）。
///
/// データベースの `day.date` などに文字列として保存する値。
/// 文字列にするときは [`Display`](fmt::Display)、読むときは [`FromStr`] を使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogicalDate(Date);

impl LogicalDate {
    /// 瞬間 `at` の論理日付を、タイムゾーン `tz` と切り替わり時刻 `boundary` から求める。
    pub fn at(at: Timestamp, tz: &TimeZone, boundary: DayBoundary) -> Self {
        let calendar_date = at.to_zoned(tz.clone()).date();
        match day_start(calendar_date, tz, boundary) {
            Some(start) if at < start => Self(prev_day(calendar_date)),
            // 求められないのは、暦の端（-9999年・9999年）だけ。カレンダー日付のままにする
            _ => Self(calendar_date),
        }
    }

    /// 時刻付きの値（タイムゾーンを含む）の論理日付を求める。
    pub fn of_zoned(zoned: &Zoned, boundary: DayBoundary) -> Self {
        Self::at(zoned.timestamp(), zoned.time_zone(), boundary)
    }

    /// 年・月・日から作る。存在しない日付（例 2月30日）なら `None`。
    pub fn new(year: i16, month: i8, day: i8) -> Option<Self> {
        Date::new(year, month, day).ok().map(Self)
    }

    /// 前日の論理日付。
    pub fn yesterday(self) -> Self {
        Self(prev_day(self.0))
    }

    /// jiff の日付として取り出す（日付の加減算などに使う）。
    pub fn as_date(self) -> Date {
        self.0
    }
}

impl From<LogicalDate> for Date {
    fn from(d: LogicalDate) -> Self {
        d.0
    }
}

impl fmt::Display for LogicalDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}",
            self.0.year(),
            self.0.month(),
            self.0.day()
        )
    }
}

impl FromStr for LogicalDate {
    type Err = ParseError;

    /// `YYYY-MM-DD`（年は4桁、月・日は2桁）だけを受け付ける。存在しない日付は誤り。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseError::Date(s.to_owned());
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return Err(err());
        }
        let year = four_digits(&b[0..4]).ok_or_else(err)?;
        let month = two_digits(&b[5..7]).ok_or_else(err)?;
        let day = two_digits(&b[8..10]).ok_or_else(err)?;
        Self::new(year as i16, month as i8, day as i8).ok_or_else(err)
    }
}

/// CLI・MCP の日付の指定（`--date D`）。`today`・`yesterday`・`YYYY-MM-DD` を受け付ける
/// （docs/design/05-mcp-cli-spec.md 3章）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DateSpec {
    /// 論理日付の今日。指定を省いたときもこれ。
    #[default]
    Today,
    /// 論理日付の昨日（今日の前日）。
    Yesterday,
    /// 日付をそのまま指定したもの。
    Date(LogicalDate),
}

impl DateSpec {
    /// 論理日付の今日 `today` を基準に、指定を論理日付に直す。
    pub fn resolve(self, today: LogicalDate) -> LogicalDate {
        match self {
            Self::Today => today,
            Self::Yesterday => today.yesterday(),
            Self::Date(d) => d,
        }
    }

    /// 瞬間 `now` の論理日付を今日として、指定を論理日付に直す。
    pub fn resolve_at(self, now: Timestamp, tz: &TimeZone, boundary: DayBoundary) -> LogicalDate {
        self.resolve(LogicalDate::at(now, tz, boundary))
    }
}

impl FromStr for DateSpec {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "today" => Ok(Self::Today),
            "yesterday" => Ok(Self::Yesterday),
            _ => s
                .parse()
                .map(Self::Date)
                .map_err(|_| ParseError::DateSpec(s.to_owned())),
        }
    }
}

/// 文字列を読めなかったときの誤り。読めなかった文字列を持つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// 切り替わり時刻が `HH:MM`（00:00〜23:59）でない。
    DayBoundary(String),
    /// 日付が `YYYY-MM-DD` でない、または存在しない日付。
    Date(String),
    /// 日付の指定が `today`・`yesterday`・`YYYY-MM-DD` のどれでもない。
    DateSpec(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DayBoundary(s) => {
                write!(
                    f,
                    "invalid day boundary {s:?}: expected HH:MM (00:00-23:59)"
                )
            }
            Self::Date(s) => write!(f, "invalid date {s:?}: expected YYYY-MM-DD"),
            Self::DateSpec(s) => {
                write!(
                    f,
                    "invalid date {s:?}: expected YYYY-MM-DD, today or yesterday"
                )
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// 端末のタイムゾーン。決められなければ UTC（jiff の既定の扱い）。
///
/// `day.tz` に保存する名前は [`TimeZone::iana_name`] で得る。
pub fn system_time_zone() -> TimeZone {
    TimeZone::system()
}

/// カレンダー日 `date` の論理日付が始まる瞬間（その日の切り替わり時刻）。
///
/// - 切り替わり時刻が夏時間の開始で飛ばされるなら、飛ばされた後の最初の瞬間
///   （時刻の切り替えの瞬間）。jiff の `compatible` は時差の分だけ後ろへずらす
///   （2:30 が無ければ 3:30）が、壁時計で 3:00〜3:29 を「2:30 を過ぎた」と扱うため、
///   切り替えの瞬間を使う。
/// - 2回あるなら、1回目。
fn day_start(date: Date, tz: &TimeZone, boundary: DayBoundary) -> Option<Timestamp> {
    let dt = date.to_datetime(boundary.to_time());
    match tz.to_ambiguous_timestamp(dt).offset() {
        AmbiguousOffset::Unambiguous { offset } => offset.to_timestamp(dt).ok(),
        AmbiguousOffset::Fold { before, .. } => before.to_timestamp(dt).ok(),
        AmbiguousOffset::Gap { after, .. } => {
            // 飛ばされた時刻を切り替え後の時差で読むと、切り替えの瞬間より前になる。
            // そこから次の切り替えが、飛ばされた時間帯の終わり
            let in_gap = after.to_timestamp(dt).ok()?;
            tz.following(in_gap).next().map(|t| t.timestamp())
        }
    }
}

/// 前日。暦の最初の日（-9999-01-01）だけはそのまま返す。
fn prev_day(d: Date) -> Date {
    d.yesterday().unwrap_or(d)
}

fn two_digits(b: &[u8]) -> Option<u8> {
    match b {
        [h @ b'0'..=b'9', l @ b'0'..=b'9'] => Some((h - b'0') * 10 + (l - b'0')),
        _ => None,
    }
}

fn four_digits(b: &[u8]) -> Option<u16> {
    let (hi, lo) = (two_digits(&b[0..2])?, two_digits(&b[2..4])?);
    Some(u16::from(hi) * 100 + u16::from(lo))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tz(name: &str) -> TimeZone {
        TimeZone::get(name).unwrap_or_else(|e| panic!("time zone {name}: {e}"))
    }

    fn boundary(s: &str) -> DayBoundary {
        s.parse().unwrap()
    }

    fn date(s: &str) -> LogicalDate {
        s.parse().unwrap()
    }

    /// `2026-11-02T03:59:00[Asia/Tokyo]` のような時刻付きの文字列の論理日付
    fn logical(zoned: &str, b: &str) -> String {
        let z: Zoned = zoned.parse().unwrap();
        LogicalDate::of_zoned(&z, boundary(b)).to_string()
    }

    // --- 切り替わり時刻をまたぐ ---

    #[test]
    fn before_boundary_is_previous_day() {
        assert_eq!(
            logical("2026-11-02T03:59:00[Asia/Tokyo]", "04:00"),
            "2026-11-01"
        );
        assert_eq!(
            logical("2026-11-02T03:59:59.999[Asia/Tokyo]", "04:00"),
            "2026-11-01"
        );
        assert_eq!(
            logical("2026-11-02T00:00:00[Asia/Tokyo]", "04:00"),
            "2026-11-01"
        );
    }

    #[test]
    fn at_or_after_boundary_is_same_day() {
        assert_eq!(
            logical("2026-11-02T04:00:00[Asia/Tokyo]", "04:00"),
            "2026-11-02"
        );
        assert_eq!(
            logical("2026-11-02T04:01:00[Asia/Tokyo]", "04:00"),
            "2026-11-02"
        );
        assert_eq!(
            logical("2026-11-02T23:59:59[Asia/Tokyo]", "04:00"),
            "2026-11-02"
        );
    }

    #[test]
    fn crosses_month_and_year() {
        assert_eq!(
            logical("2026-03-01T01:00:00[Asia/Tokyo]", "04:00"),
            "2026-02-28"
        );
        assert_eq!(
            logical("2028-03-01T01:00:00[Asia/Tokyo]", "04:00"),
            "2028-02-29"
        );
        assert_eq!(
            logical("2027-01-01T03:00:00[Asia/Tokyo]", "04:00"),
            "2026-12-31"
        );
    }

    #[test]
    fn midnight_boundary_is_calendar_date() {
        assert_eq!(
            logical("2026-11-02T00:00:00[Asia/Tokyo]", "00:00"),
            "2026-11-02"
        );
        assert_eq!(
            logical("2026-11-01T23:59:59[Asia/Tokyo]", "00:00"),
            "2026-11-01"
        );
    }

    #[test]
    fn boundary_with_minutes() {
        assert_eq!(
            logical("2026-11-02T05:29:00[Asia/Tokyo]", "05:30"),
            "2026-11-01"
        );
        assert_eq!(
            logical("2026-11-02T05:30:00[Asia/Tokyo]", "05:30"),
            "2026-11-02"
        );
    }

    #[test]
    fn same_instant_depends_on_time_zone() {
        // 2026-11-01T19:30Z は東京では 11/2 4:30、ロンドンでは 11/1 19:30
        let at: Timestamp = "2026-11-01T19:30:00Z".parse().unwrap();
        let b = DayBoundary::DEFAULT;
        assert_eq!(
            LogicalDate::at(at, &tz("Asia/Tokyo"), b),
            date("2026-11-02")
        );
        assert_eq!(
            LogicalDate::at(at, &tz("Europe/London"), b),
            date("2026-11-01")
        );
        // 2026-11-01T18:30Z は東京では 11/2 3:30 で、まだ 11/1
        let at: Timestamp = "2026-11-01T18:30:00Z".parse().unwrap();
        assert_eq!(
            LogicalDate::at(at, &tz("Asia/Tokyo"), b),
            date("2026-11-01")
        );
    }

    // --- 夏時間の切り替え ---
    // America/New_York：2026-03-08 は 2:00→3:00（2:00〜2:59 が無い）、
    // 2026-11-01 は 2:00→1:00（1:00〜1:59 が2回ある）

    #[test]
    fn dst_default_boundary_unaffected() {
        // 4:00 は切り替えの影響を受けない。その日は 23 時間・25 時間になる
        let ny = "America/New_York";
        assert_eq!(
            logical(&format!("2026-03-08T03:59:00[{ny}]"), "04:00"),
            "2026-03-07"
        );
        assert_eq!(
            logical(&format!("2026-03-08T04:00:00[{ny}]"), "04:00"),
            "2026-03-08"
        );
        assert_eq!(
            logical(&format!("2026-11-01T03:59:00[{ny}]"), "04:00"),
            "2026-10-31"
        );
        assert_eq!(
            logical(&format!("2026-11-01T04:00:00[{ny}]"), "04:00"),
            "2026-11-01"
        );
    }

    #[test]
    fn dst_boundary_in_gap_starts_after_gap() {
        // 2:30 が無い日は、飛ばされた後の 3:00 にその日が始まる
        let ny = "America/New_York";
        assert_eq!(
            logical(&format!("2026-03-08T01:59:59[{ny}]"), "02:30"),
            "2026-03-07"
        );
        assert_eq!(
            logical(&format!("2026-03-08T03:00:00[{ny}]"), "02:30"),
            "2026-03-08"
        );
        // 3:00〜3:29 は、壁時計では 2:30 前ではないので当日（3:00 を過ぎている）
        assert_eq!(
            logical(&format!("2026-03-08T03:15:00[{ny}]"), "02:30"),
            "2026-03-08"
        );
        // 前後の日はふだんどおり
        assert_eq!(
            logical(&format!("2026-03-09T02:29:00[{ny}]"), "02:30"),
            "2026-03-08"
        );
        assert_eq!(
            logical(&format!("2026-03-09T02:30:00[{ny}]"), "02:30"),
            "2026-03-09"
        );
    }

    #[test]
    fn dst_boundary_in_fold_starts_at_first_occurrence_and_never_goes_back() {
        let ny = tz("America/New_York");
        let b = boundary("01:30");
        // 1回目の 1:30（夏時間 -04:00）＝ 05:30Z に始まる
        let first_0129: Timestamp = "2026-11-01T05:29:00Z".parse().unwrap();
        let first_0130: Timestamp = "2026-11-01T05:30:00Z".parse().unwrap();
        assert_eq!(LogicalDate::at(first_0129, &ny, b), date("2026-10-31"));
        assert_eq!(LogicalDate::at(first_0130, &ny, b), date("2026-11-01"));
        // 2回目の 1:00〜1:59（標準時 -05:00）＝ 06:00Z〜06:59Z。
        // 壁時計では 1:30 より前の時刻もあるが、前日には戻らない
        for minute in [0, 15, 29, 30, 59] {
            let at: Timestamp = format!("2026-11-01T06:{minute:02}:00Z").parse().unwrap();
            assert_eq!(
                LogicalDate::at(at, &ny, b),
                date("2026-11-01"),
                "06:{minute:02}Z"
            );
        }
    }

    #[test]
    fn dst_dates_never_go_back_across_transitions() {
        // 夏時間の切り替えの前後を 1 分ずつ進め、論理日付が減らないことを確かめる
        let ny = tz("America/New_York");
        for (start, b) in [
            ("2026-03-07T12:00:00Z", "02:30"),
            ("2026-03-07T12:00:00Z", "02:00"),
            ("2026-03-07T12:00:00Z", "04:00"),
            ("2026-10-31T12:00:00Z", "01:30"),
            ("2026-10-31T12:00:00Z", "01:00"),
            ("2026-10-31T12:00:00Z", "02:00"),
        ] {
            let b = boundary(b);
            let mut at: Timestamp = start.parse().unwrap();
            let mut prev = LogicalDate::at(at, &ny, b);
            let mut changes = 0;
            for _ in 0..(48 * 60) {
                at = at.checked_add(jiff::SignedDuration::from_mins(1)).unwrap();
                let cur = LogicalDate::at(at, &ny, b);
                assert!(cur >= prev, "went back at {at} with boundary {b}");
                if cur != prev {
                    assert_eq!(cur, prev.as_date().tomorrow().map(LogicalDate).unwrap());
                    changes += 1;
                }
                prev = cur;
            }
            assert_eq!(changes, 2, "48 hours from {start} with boundary {b}");
        }
    }

    // --- 切り替わり時刻を変えた後 ---

    #[test]
    fn changing_boundary_applies_only_to_new_dates() {
        let tokyo = tz("Asia/Tokyo");
        // 4:00 のとき、11/2 5:00 の記録は 11/2 に入る
        let recorded_at: Timestamp = "2026-11-02T05:00:00+09:00".parse().unwrap();
        let stored = LogicalDate::at(recorded_at, &tokyo, boundary("04:00")).to_string();
        assert_eq!(stored, "2026-11-02");

        // 切り替わり時刻を 6:00 に変えた後、同じ日の 5:30 の今日は 11/1 になる
        let now: Timestamp = "2026-11-02T05:30:00+09:00".parse().unwrap();
        let b = boundary("06:00");
        assert_eq!(
            DateSpec::Today.resolve_at(now, &tokyo, b),
            date("2026-11-01")
        );

        // 保存済みの日付は値として読み戻すだけで、付け直さない
        assert_eq!(date(&stored), date("2026-11-02"));
        // 新しく求める日付にだけ、変えた後の切り替わり時刻が効く
        let recorded_after: Timestamp = "2026-11-03T05:00:00+09:00".parse().unwrap();
        assert_eq!(
            LogicalDate::at(recorded_after, &tokyo, b),
            date("2026-11-02")
        );
        let recorded_after: Timestamp = "2026-11-03T06:00:00+09:00".parse().unwrap();
        assert_eq!(
            LogicalDate::at(recorded_after, &tokyo, b),
            date("2026-11-03")
        );
    }

    // --- 日付の指定（today / yesterday / YYYY-MM-DD） ---

    #[test]
    fn date_spec_parses_and_resolves() {
        let today = date("2026-03-01");
        let spec = |s: &str| s.parse::<DateSpec>().unwrap();
        assert_eq!(spec("today").resolve(today), date("2026-03-01"));
        assert_eq!(spec("yesterday").resolve(today), date("2026-02-28"));
        assert_eq!(spec("2026-10-31").resolve(today), date("2026-10-31"));
        assert_eq!(DateSpec::default().resolve(today), today);
    }

    #[test]
    fn date_spec_today_uses_logical_date() {
        // 深夜 3:59 の today は前日、yesterday は前々日
        let tokyo = tz("Asia/Tokyo");
        let now: Timestamp = "2026-11-02T03:59:00+09:00".parse().unwrap();
        let b = DayBoundary::DEFAULT;
        assert_eq!(
            DateSpec::Today.resolve_at(now, &tokyo, b),
            date("2026-11-01")
        );
        assert_eq!(
            DateSpec::Yesterday.resolve_at(now, &tokyo, b),
            date("2026-10-31")
        );
    }

    #[test]
    fn date_spec_rejects_other_forms() {
        for s in [
            "",
            "Today",
            "TODAY",
            " today",
            "tomorrow",
            "昨日",
            "2026-1-02",
            "2026/11/02",
            "20261102",
            "2026-11-02T00:00",
            "+2026-11-02",
            "2026-02-30",
            "2026-13-01",
            "2026-00-10",
            "2026-11-00",
        ] {
            assert_eq!(
                s.parse::<DateSpec>(),
                Err(ParseError::DateSpec(s.to_owned())),
                "{s:?}"
            );
        }
    }

    #[test]
    fn logical_date_round_trips() {
        for s in ["2026-11-02", "2028-02-29", "0001-01-01", "9999-12-31"] {
            assert_eq!(date(s).to_string(), s);
        }
        assert!("2027-02-29".parse::<LogicalDate>().is_err());
    }

    #[test]
    fn day_boundary_parses_and_formats() {
        assert_eq!(DayBoundary::default(), boundary("04:00"));
        assert_eq!(boundary("00:00").to_string(), "00:00");
        assert_eq!(boundary("23:59").to_string(), "23:59");
        assert_eq!(boundary("05:30"), DayBoundary::new(5, 30).unwrap());
        for s in [
            "24:00", "04:60", "4:00", "04:0", "0400", "04-00", "", " 04:00", "04:00:00",
        ] {
            assert!(s.parse::<DayBoundary>().is_err(), "{s:?}");
        }
        assert_eq!(DayBoundary::new(24, 0), None);
    }
}
