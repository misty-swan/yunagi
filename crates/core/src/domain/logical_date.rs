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
//! - 切り替わり時刻が夏時間の開始で飛ばされる日（例：2:00〜2:59 が無い日に 2:30）は、
//!   飛ばされた後の最初の時刻（この例では 3:00）にその日が始まる。
//! - 切り替わり時刻が夏時間の終了で2回ある日（例：1:00〜1:59 が2回ある日に 1:30）は、
//!   1回目にその日が始まる。2回目の 1:00〜1:30 も、もう同じ日である。
//!
//! # 切り替わり時刻を変えたとき
//!
//! この規則は、新しく日付を求めるときにだけ使う。保存済みの日付（[`LogicalDate`]）は
//! 値として持つだけで、切り替わり時刻を変えても付け直さない（アーキテクチャ設計書 5章）。
//! そのため、このモジュールには保存済みの日付を切り替わり時刻から求め直す関数を置かない。
//!
//! # タイムゾーン
//!
//! 端末のタイムゾーンは `TimeZone::try_system()` で得る。`TimeZone::system()` は
//! 決められないとき黙って UTC にし、日付が気づかれずにずれるので使わない。
//!
//! 日付の行（`day.tz`）に記録するタイムゾーンの文字列は [`time_zone_record`] で作り、
//! [`parse_time_zone_record`] で読み戻す（データモデル設計書 3章 day）。

use std::fmt;
use std::str::FromStr;

use jiff::civil::{Date, Time};
use jiff::fmt::temporal::{DateTimeParser, DateTimePrinter};
use jiff::tz::{AmbiguousOffset, TimeZone};
use jiff::{Span, Timestamp, Zoned};

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
    ///
    /// 1〜9999年の外になる瞬間は、その端の日（0001-01-01・9999-12-31）に収める。
    pub fn at(at: Timestamp, tz: &TimeZone, boundary: DayBoundary) -> Self {
        let calendar_date = tz.to_datetime(at).date();
        // その日の始まりを求められないのは、暦の端（-9999年・9999年）か、夏時間の開始の後に
        // 次の切り替えが無い壊れたタイムゾーンのデータのとき。カレンダー日付のままにする
        let date = match day_start(calendar_date, tz, boundary) {
            Some(start) if at < start => calendar_date.yesterday().unwrap_or(calendar_date),
            // 翌日がもう始まっていれば翌日。夏時間の終了で時計が深夜0時をまたいで前日へ戻る
            // タイムゾーンでは、カレンダー日付が戻っても翌日はすでに始まっている
            _ => match calendar_date.tomorrow() {
                Ok(next) if day_start(next, tz, boundary).is_some_and(|start| start <= at) => next,
                _ => calendar_date,
            },
        };
        Self(date).clamp(Self::MIN, Self::MAX)
    }

    /// 扱える最初の日（0001-01-01）。
    pub const MIN: Self = Self(Date::constant(1, 1, 1));
    /// 扱える最後の日（9999-12-31）。
    pub const MAX: Self = Self(Date::constant(9999, 12, 31));

    /// 時刻付きの値（タイムゾーンを含む）の論理日付を求める。
    pub fn of_zoned(zoned: &Zoned, boundary: DayBoundary) -> Self {
        Self::at(zoned.timestamp(), zoned.time_zone(), boundary)
    }

    /// 年・月・日から作る。存在しない日付（例 2月30日）や、
    /// `YYYY-MM-DD` で書けない年（1〜9999年の外）なら `None`。
    pub fn new(year: i16, month: i8, day: i8) -> Option<Self> {
        if !(1..=9999).contains(&year) {
            return None;
        }
        Date::new(year, month, day).ok().map(Self)
    }

    /// 翌日の論理日付。9999-12-31 の翌日は無いので `None`。
    pub fn tomorrow(self) -> Option<Self> {
        (self < Self::MAX)
            .then(|| self.0.tomorrow().ok().map(Self))
            .flatten()
    }

    /// 前日の論理日付。0001-01-01 の前日は無いので `None`。
    pub fn yesterday(self) -> Option<Self> {
        (self > Self::MIN)
            .then(|| self.0.yesterday().ok().map(Self))
            .flatten()
    }

    /// `days` 日後（負なら前）の論理日付。0001-01-01〜9999-12-31 の外になるなら `None`。
    pub fn add_days(self, days: i32) -> Option<Self> {
        // jiff の日数の範囲（およそ ±2万年）を超える値は、Span を作る時点で断る
        let span = Span::new().try_days(days).ok()?;
        self.0
            .checked_add(span)
            .ok()
            .map(Self)
            .filter(|d| (Self::MIN..=Self::MAX).contains(d))
    }

    /// jiff の日付として取り出す（曜日を見るなど）。
    pub fn as_date(self) -> Date {
        self.0
    }
}

impl fmt::Display for LogicalDate {
    /// `YYYY-MM-DD`。値は 1〜9999年に限るので、いつも [`FromStr`] で読み戻せる。
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for LogicalDate {
    type Err = ParseError;

    /// `YYYY-MM-DD`（年は4桁、月・日は2桁）だけを受け付ける。存在しない日付は誤り。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_ymd(s).map_err(|_| ParseError::Date(s.to_owned()))
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
    /// 今日が 0001-01-01 のときの `yesterday` だけは日付が無いので `None`。
    pub fn resolve(self, today: LogicalDate) -> Option<LogicalDate> {
        match self {
            Self::Today => Some(today),
            Self::Yesterday => today.yesterday(),
            Self::Date(d) => Some(d),
        }
    }

    /// 瞬間 `now` の論理日付を今日として、指定を論理日付に直す。
    pub fn resolve_at(
        self,
        now: Timestamp,
        tz: &TimeZone,
        boundary: DayBoundary,
    ) -> Option<LogicalDate> {
        self.resolve(LogicalDate::at(now, tz, boundary))
    }
}

impl FromStr for DateSpec {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "today" => Ok(Self::Today),
            "yesterday" => Ok(Self::Yesterday),
            // 形は YYYY-MM-DD だが存在しない日付（2026-02-30 など）は、ParseError::Date で返す
            _ => parse_ymd(s).map(Self::Date).map_err(|e| match e {
                YmdError::Shape => ParseError::DateSpec(s.to_owned()),
                YmdError::Nonexistent => ParseError::Date(s.to_owned()),
            }),
        }
    }
}

/// 日付の行（`day.tz`）に記録する、タイムゾーン `tz` の文字列（データモデル設計書 3章 day）。
///
/// 次のうち最初に当てはまる形にする。どれも [`parse_time_zone_record`] で読み戻せる。
///
/// 1. IANA の名前が取れれば、その名前（`Asia/Tokyo`、`UTC`）。zoneinfo の中の別の版のフォルダ
///    （`posix/`・`right/`。`/etc/localtime` が `/usr/share/zoneinfo/posix/Asia/Tokyo` へのリンクのときなど）
///    の名前は、接頭辞を除いた名前にする（ほかの環境のタイムゾーンデータベースには、接頭辞の付いた
///    名前が無いことがあるため）。除いた名前がタイムゾーンデータベースに無ければ、4 と同じく
///    瞬間 `at` の UTC からのずれにする。
/// 2. POSIX 形式の TZ 指定（環境変数 `TZ=JST-9` など）なら、その文字列を jiff が整えた形
///    （`JST-9`、`EST5EDT,M3.2.0,M11.1.0`）。
/// 3. UTC からのずれが固定なら、そのずれ（`+09:00`。秒まであれば `-00:44:30`）。
/// 4. どれでもない（名前の無いタイムゾーンデータ。`/etc/localtime` が zoneinfo へのリンクでない、
///    `TZ` が zoneinfo の外のファイルを指す、など）なら、瞬間 `at` の UTC からのずれ（3 と同じ形）。
///    読み戻せるのはそのずれだけで、夏時間などの規則は残らない。読み戻したもので同じに求まるのは
///    `at` の壁時計の時刻で、論理日付は、その日の始まりから `at` までに夏時間の切り替えがあると
///    違いうる（どの固定のずれでも、すべての瞬間の論理日付は再現できない）。
///
/// 1〜3 は jiff の `DateTimePrinter::time_zone_to_string` の形で、4 はそれが文字列にできない
/// ときの代わり。`TimeZone::unknown()` は `Etc/Unknown` になる（[`TimeZone::try_system`] は返さない）。
pub fn time_zone_record(tz: &TimeZone, at: Timestamp) -> String {
    static PRINTER: DateTimePrinter = DateTimePrinter::new();
    let offset_at = || {
        PRINTER
            .time_zone_to_string(&TimeZone::fixed(tz.to_offset(at)))
            .expect("a fixed offset time zone is always printable")
    };
    if let Some(name) = tz.iana_name()
        && let Some(rest) = strip_zoneinfo_variant(name)
    {
        // TimeZone::get は大文字・小文字を区別せず、データベースでの書き方の名前を返す
        return match TimeZone::get(rest) {
            Ok(zone) => zone.iana_name().unwrap_or(rest).to_owned(),
            Err(_) => offset_at(),
        };
    }
    PRINTER
        .time_zone_to_string(tz)
        .unwrap_or_else(|_| offset_at())
}

/// zoneinfo の中の別の版のフォルダ（`posix/`・`right/`）の名前から、接頭辞を除いた名前。
/// 接頭辞が無ければ `None`。大文字・小文字は区別しない（IANA の名前の引き方と同じ）。
fn strip_zoneinfo_variant(name: &str) -> Option<&str> {
    ["posix/", "right/"].iter().find_map(|prefix| {
        name.get(..prefix.len())
            .filter(|head| head.eq_ignore_ascii_case(prefix))
            .map(|_| &name[prefix.len()..])
    })
}

/// [`time_zone_record`] で記録した文字列を、タイムゾーンに読み戻す。
///
/// IANA の名前・UTC からのずれ（`+09:00`）・POSIX 形式の TZ 指定を受け付ける。
/// IANA の名前は、いまのタイムゾーンデータベースで引く（無い名前は誤り）。
pub fn parse_time_zone_record(s: &str) -> Result<TimeZone, ParseError> {
    static PARSER: DateTimeParser = DateTimeParser::new();
    PARSER
        .parse_time_zone(s)
        .map_err(|_| ParseError::TimeZone(s.to_owned()))
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
    /// タイムゾーンが、IANA の名前・UTC からのずれ・POSIX 形式の TZ 指定のどれでもない。
    TimeZone(String),
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
            Self::TimeZone(s) => write!(
                f,
                "invalid time zone {s:?}: expected an IANA name, a UTC offset or a POSIX TZ string"
            ),
        }
    }
}

impl std::error::Error for ParseError {}

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

/// `YYYY-MM-DD` を読めなかった理由。
enum YmdError {
    /// 形が `YYYY-MM-DD`（年は4桁、月・日は2桁の数字）でない。
    Shape,
    /// 形は合っているが、存在しない日付（2026-02-30、0000年など）。
    Nonexistent,
}

/// `YYYY-MM-DD` を読む。[`LogicalDate`] と [`DateSpec`] の読み取りはここだけを使う。
fn parse_ymd(s: &str) -> Result<LogicalDate, YmdError> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return Err(YmdError::Shape);
    }
    let year = four_digits(&b[0..4]).ok_or(YmdError::Shape)?;
    let month = two_digits(&b[5..7]).ok_or(YmdError::Shape)?;
    let day = two_digits(&b[8..10]).ok_or(YmdError::Shape)?;
    LogicalDate::new(year as i16, month as i8, day as i8).ok_or(YmdError::Nonexistent)
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

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    /// `2026-11-02T03:59:00[Asia/Tokyo]` のような時刻付きの文字列の論理日付
    fn logical(zoned: &str, b: &str) -> String {
        let z: Zoned = zoned.parse().unwrap();
        LogicalDate::of_zoned(&z, boundary(b)).to_string()
    }

    // --- 切り替わり時刻をまたぐ ---

    #[test]
    fn add_days_moves_within_the_calendar() {
        assert_eq!(date("2026-11-03").add_days(0), Some(date("2026-11-03")));
        assert_eq!(date("2026-12-30").add_days(3), Some(date("2027-01-02")));
        assert_eq!(date("2026-03-01").add_days(-1), Some(date("2026-02-28")));
        assert_eq!(LogicalDate::MAX.add_days(1), None);
        assert_eq!(LogicalDate::MIN.add_days(-1), None);
        assert_eq!(LogicalDate::MIN.add_days(i32::MAX), None);
        assert_eq!(LogicalDate::MAX.add_days(i32::MIN), None);
    }

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
                    assert_eq!(Some(cur), prev.tomorrow());
                    changes += 1;
                }
                prev = cur;
            }
            assert_eq!(changes, 2, "48 hours from {start} with boundary {b}");
        }
    }

    #[test]
    fn dst_fold_across_midnight_does_not_go_back() {
        // 夏時間が 0:30 に終わり、前日の 23:30 に戻る規則（POSIX 形式。実在の例は少ないので作る）。
        // 2026-10-25 0:00（+04:00）＝ 2026-10-24T20:00Z に 10/25 が始まり、
        // 0:30（20:30Z）に時計が 10/24 23:30（+03:00）へ戻っても 10/25 のまま
        let zone = TimeZone::posix("XST-3XDT,M3.5.0/0,M10.5.0/0:30").unwrap();
        let b = boundary("00:00");
        let at = |s: &str| s.parse::<Timestamp>().unwrap();
        for (utc, expected) in [
            ("2026-10-24T19:59:00Z", "2026-10-24"),
            ("2026-10-24T20:00:00Z", "2026-10-25"),
            ("2026-10-24T20:30:00Z", "2026-10-25"), // 壁時計は 10/24 23:30
            ("2026-10-24T20:59:00Z", "2026-10-25"), // 壁時計は 10/24 23:59
            ("2026-10-24T21:00:00Z", "2026-10-25"),
        ] {
            assert_eq!(LogicalDate::at(at(utc), &zone, b), date(expected), "{utc}");
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
            Some(date("2026-11-01"))
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
        assert_eq!(spec("today").resolve(today), Some(date("2026-03-01")));
        assert_eq!(spec("yesterday").resolve(today), Some(date("2026-02-28")));
        assert_eq!(spec("2026-10-31").resolve(today), Some(date("2026-10-31")));
        assert_eq!(DateSpec::default().resolve(today), Some(today));
        assert_eq!(spec("yesterday").resolve(LogicalDate::MIN), None);
    }

    #[test]
    fn date_spec_today_uses_logical_date() {
        // 深夜 3:59 の today は前日、yesterday は前々日
        let tokyo = tz("Asia/Tokyo");
        let now: Timestamp = "2026-11-02T03:59:00+09:00".parse().unwrap();
        let b = DayBoundary::DEFAULT;
        assert_eq!(
            DateSpec::Today.resolve_at(now, &tokyo, b),
            Some(date("2026-11-01"))
        );
        assert_eq!(
            DateSpec::Yesterday.resolve_at(now, &tokyo, b),
            Some(date("2026-10-31"))
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
            "2026-11-0a",
        ] {
            assert_eq!(
                s.parse::<DateSpec>(),
                Err(ParseError::DateSpec(s.to_owned())),
                "{s:?}"
            );
        }
    }

    #[test]
    fn date_spec_reports_nonexistent_date_as_date_error() {
        for s in [
            "2026-02-30",
            "2026-13-01",
            "2026-00-10",
            "2026-11-00",
            "0000-01-01",
        ] {
            assert_eq!(
                s.parse::<DateSpec>(),
                Err(ParseError::Date(s.to_owned())),
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
        assert!("0000-12-31".parse::<LogicalDate>().is_err());
        assert_eq!(LogicalDate::new(0, 1, 1), None);
        assert_eq!(LogicalDate::new(-1, 1, 1), None);
        assert_eq!(date("2026-12-31").tomorrow(), Some(date("2027-01-01")));
        assert_eq!(date("9999-12-31").tomorrow(), None);
        assert_eq!(date("0001-01-01").yesterday(), None);
        // 1〜9999年の外の瞬間は端の日に収め、読み戻せる文字列だけを作る
        let b = DayBoundary::DEFAULT;
        assert_eq!(
            LogicalDate::at(Timestamp::MIN, &TimeZone::UTC, b),
            LogicalDate::MIN
        );
        assert_eq!(LogicalDate::MIN.to_string(), "0001-01-01");
        // jiff の最後の瞬間は 9999-12-30 のうち。どのタイムゾーンでも読み戻せる日付になる
        for zone in [TimeZone::UTC, tz("Asia/Tokyo"), tz("Pacific/Kiritimati")] {
            let last = LogicalDate::at(Timestamp::MAX, &zone, b);
            assert_eq!(last.to_string().parse(), Ok(last));
        }
    }

    // --- day.tz に記録するタイムゾーン ---

    #[test]
    fn time_zone_record_forms_and_round_trips() {
        let at = ts("2026-07-01T00:00:00Z");
        for (zone, expected) in [
            (tz("Asia/Tokyo"), "Asia/Tokyo"),
            (tz("america/NEW_YORK"), "America/New_York"),
            (TimeZone::UTC, "UTC"),
            (TimeZone::posix("JST-9").unwrap(), "JST-9"),
            (
                TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap(),
                "EST5EDT,M3.2.0,M11.1.0",
            ),
            (TimeZone::fixed(jiff::tz::offset(9)), "+09:00"),
            (
                TimeZone::fixed(jiff::tz::Offset::from_seconds(-(44 * 60 + 30)).unwrap()),
                "-00:44:30",
            ),
            (TimeZone::unknown(), "Etc/Unknown"),
        ] {
            let record = time_zone_record(&zone, at);
            assert_eq!(record, expected);
            // 読み戻すと同じタイムゾーンになり、もう一度記録しても同じ文字列になる
            let restored = parse_time_zone_record(&record).unwrap();
            assert_eq!(restored, zone, "{record}");
            assert_eq!(time_zone_record(&restored, at), record);
        }
    }

    #[test]
    fn restored_time_zone_gives_the_same_logical_dates() {
        // 名前の無い POSIX 形式でも、読み戻したもので夏時間の前後の論理日付が同じになる
        let b = boundary("01:30");
        for zone in [
            tz("America/New_York"),
            TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap(),
        ] {
            let restored =
                parse_time_zone_record(&time_zone_record(&zone, ts("2026-07-01T00:00:00Z")))
                    .unwrap();
            // 夏時間の開始・終了をはさむ 3 日ずつを、15分ずつ
            for (start, end) in [
                ("2026-03-07T00:00:00Z", "2026-03-10T00:00:00Z"),
                ("2026-10-31T00:00:00Z", "2026-11-03T00:00:00Z"),
            ] {
                let mut t = ts(start);
                while t < ts(end) {
                    assert_eq!(
                        LogicalDate::at(t, &restored, b),
                        LogicalDate::at(t, &zone, b)
                    );
                    t = t.checked_add(jiff::SignedDuration::from_mins(15)).unwrap();
                }
            }
        }
    }

    #[test]
    fn time_zone_record_rejects_unknown_forms() {
        for s in [
            "",
            "Does/Not/Exist",
            "05:00",
            "Z",
            "+09:00:00.5",
            " Asia/Tokyo",
        ] {
            assert_eq!(
                parse_time_zone_record(s),
                Err(ParseError::TimeZone(s.to_owned())),
                "{s:?}"
            );
        }
    }

    /// 子プロセスに、記録されるはずの文字列を渡す環境変数。
    #[cfg(unix)]
    const CHILD_EXPECTED: &str = "YUNAGI_TEST_TZ_RECORD_EXPECTED";
    /// 子プロセスに、IANA の名前が取れるはずか（`1`・`0`）を渡す環境変数。
    #[cfg(unix)]
    const CHILD_NAMED: &str = "YUNAGI_TEST_TZ_RECORD_NAMED";

    /// 端末のタイムゾーン（`TimeZone::try_system()`）を、環境変数 `TZ` を変えた子プロセスで記録する。
    /// 環境変数はプロセス全体で共有され、テストは並行に走るので、このプロセスの `TZ` は変えない。
    #[cfg(unix)]
    #[test]
    fn records_system_time_zone_without_iana_name() {
        // zoneinfo の外にコピーした TZif ファイルは、名前の無いタイムゾーンとして読まれる
        let dir = tempfile::tempdir().unwrap();
        let unnamed = dir.path().join("localtime");
        std::fs::copy(system_tzif_path("America/New_York"), &unnamed).unwrap();
        for (tz_env, named, expected) in [
            ("Asia/Tokyo", true, "Asia/Tokyo"),
            ("JST-9", false, "JST-9"),
            ("EST5EDT,M3.2.0,M11.1.0", false, "EST5EDT,M3.2.0,M11.1.0"),
            // 子プロセスで記録する瞬間（2026-07-01）は夏時間
            (unnamed.to_str().unwrap(), false, "-04:00"),
        ] {
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &child_test_name(), "--test-threads=1"])
                .env("TZ", tz_env)
                .env(CHILD_EXPECTED, expected)
                .env(CHILD_NAMED, if named { "1" } else { "0" })
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("1 passed"),
                "TZ={tz_env}\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    /// システムのタイムゾーンデータの、`name` の TZif ファイルのパス。ほかのテストの
    /// `tz("Asia/Tokyo")` などと同じく、システムのタイムゾーンデータを前提にする。
    /// 探す場所は jiff と同じ（環境変数 TZDIR、なければ決まったフォルダ）。
    #[cfg(unix)]
    fn system_tzif_path(name: &str) -> std::path::PathBuf {
        std::env::var_os("TZDIR")
            .map(std::path::PathBuf::from)
            .into_iter()
            .chain(
                [
                    "/usr/share/zoneinfo",
                    "/usr/share/lib/zoneinfo",
                    "/etc/zoneinfo",
                ]
                .map(std::path::PathBuf::from),
            )
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
            .unwrap_or_else(|| panic!("needs the system tzdata ({name})"))
    }

    #[cfg(unix)]
    #[test]
    fn zoneinfo_variant_folders_are_recorded_without_their_prefix() {
        let at = ts("2026-07-01T00:00:00Z");
        let data = std::fs::read(system_tzif_path("America/New_York")).unwrap();
        for (name, expected) in [
            ("posix/America/New_York", "America/New_York"),
            ("right/America/New_York", "America/New_York"),
            ("POSIX/america/new_york", "America/New_York"),
            // 接頭辞を除いた名前が無ければ、その瞬間のずれ（夏時間）
            ("posix/Not/A_Zone", "-04:00"),
            // 接頭辞でなければ、そのまま
            ("posixrules", "posixrules"),
        ] {
            let zone = TimeZone::tzif(name, &data).unwrap();
            assert_eq!(zone.iana_name(), Some(name));
            let record = time_zone_record(&zone, at);
            assert_eq!(record, expected, "{name}");
            // 記録した名前は、接頭辞の無い名前として読み戻せる
            if record != "posixrules" {
                let restored = parse_time_zone_record(&record).unwrap();
                assert_eq!(restored.to_datetime(at), zone.to_datetime(at));
            }
        }
    }

    /// 子プロセスで走らせるテストの名前（クレート名を除いた、テストの実行時に指定する形）。
    #[cfg(unix)]
    fn child_test_name() -> String {
        let (_crate, module) = module_path!().split_once("::").unwrap();
        format!("{module}::system_time_zone_record_in_child")
    }

    /// [`records_system_time_zone_without_iana_name`] が子プロセスで走らせる。ふだんは何もしない。
    #[cfg(unix)]
    #[test]
    fn system_time_zone_record_in_child() {
        let Ok(expected) = std::env::var(CHILD_EXPECTED) else {
            return;
        };
        let zone = TimeZone::try_system().unwrap();
        let named = std::env::var(CHILD_NAMED).unwrap() == "1";
        assert_eq!(zone.iana_name().is_some(), named, "{:?}", zone.iana_name());
        let at = ts("2026-07-01T00:00:00Z");
        let record = time_zone_record(&zone, at);
        assert_eq!(record, expected);
        // 読み戻したもので、記録した瞬間の時刻と論理日付が同じになる
        let restored = parse_time_zone_record(&record).unwrap();
        assert_eq!(restored.to_datetime(at), zone.to_datetime(at));
        for b in ["00:00", "04:00", "23:59"] {
            assert_eq!(
                LogicalDate::at(at, &restored, boundary(b)),
                LogicalDate::at(at, &zone, boundary(b))
            );
        }
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
