//! 設定（docs/design/02-data-model.md 3章 setting、画面設計書 7章 S-05）。
//!
//! 設定はキーと値（JSON）の組で保存する（`storage::table::setting`）。ここで、キーごとの値の形と
//! 既定値を決め、画面が読み書きする型（[`Settings`]・[`SettingsPatch`]）にまとめる。
//!
//! - 読むとき：未設定と、形に合わない値は、既定値として扱う（壊れた設定で画面が開けなくならない
//!   ように。`day_boundary`・`carryover_review_threshold` と同じ）。
//! - 書くとき：形に合わない値は、何も書かずに断る（[`SettingsError::Invalid`]）。
//!
//! 画面（S-05）が扱うのは、日付の切り替わり時刻・凪メーターの時間帯・言語・更新の自動確認・テーマ・
//! フォントサイズ・ノートの書体・レイアウト・持ち越し見直しの回数・サイドバーの開閉・ショートカット割当・
//! Vim モード。ライセンスは、その機能を作るときに足す。

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::domain::carryover::{self, REVIEW_THRESHOLD_KEY};
use crate::domain::day::{self, DAY_BOUNDARY_KEY};
use crate::domain::logical_date::DayBoundary;
use crate::storage::table::setting;
use crate::storage::{StorageError, in_transaction};

/// 設定のキー：テーマ。
pub const THEME_KEY: &str = "theme";
/// 設定のキー：フォントサイズ（px）。
pub const FONT_SIZE_KEY: &str = "font_size";
/// 設定のキー：ノートの書体。
pub const NOTE_FONT_KEY: &str = "note_font";
/// 設定のキー：整理モードのレイアウト。
pub const LAYOUT_KEY: &str = "layout";
/// 設定のキー：画面の言語。
pub const LOCALE_KEY: &str = "locale";
/// 設定のキー：更新の自動確認。
pub const UPDATE_CHECK_KEY: &str = "update_check";
/// 設定のキー：凪メーターの時間帯（`"HH:MM-HH:MM"`）。
pub const METER_HOURS_KEY: &str = "meter_hours";
/// 設定のキー：サイドバーの開閉（広いウィンドウの整理モードでの状態）。
pub const SIDEBAR_KEY: &str = "sidebar";
/// 設定のキー：ショートカットの割当（初期割当から変えたものだけ）。
pub const SHORTCUTS_KEY: &str = "shortcuts";
/// 設定のキー：Vim モード。
pub const VIM_MODE_KEY: &str = "vim_mode";
/// 設定のキー：Vim モードの挿入モードで、直前の入力状態に戻すか。
pub const VIM_IME_RESTORE_KEY: &str = "vim_ime_restore";

/// フォントサイズの範囲（px）。
pub const FONT_SIZE_RANGE: std::ops::RangeInclusive<u8> = 13..=19;
/// フォントサイズの既定値（px）。
pub const DEFAULT_FONT_SIZE: u8 = 15;

/// 凪メーターの時間帯の既定値（始業 9:00、終業 19:00）。
pub const DEFAULT_METER_HOURS: &str = "09:00-19:00";

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            /// 設定に保存する文字列。
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }

            /// 保存した文字列から読む。知らない文字列は `None`。
            pub fn parse(s: &str) -> Option<Self> {
                match s {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

string_enum! {
    /// テーマ（既定は OS に合わせる）。
    #[derive(Default)]
    Theme {
        #[default]
        System => "system",
        Light => "light",
        Dark => "dark",
    }
}

string_enum! {
    /// ノートの書体（デザインガイド 4章。既定は本文と同じ）。
    #[derive(Default)]
    NoteFont {
        /// 本文と同じ。
        #[default]
        Body => "body",
        /// 等幅。
        Mono => "mono",
    }
}

string_enum! {
    /// 整理モードのレイアウト（画面設計書 3章「レイアウト」。既定は A）。
    #[derive(Default)]
    Layout {
        /// 2段。
        #[default]
        A => "A",
        /// 3列。
        B => "B",
        /// 縦積み。
        C => "C",
    }
}

string_enum! {
    /// 画面の言語。設定が無いときは OS の言語に合わせる（画面が決める）ので、`Option<Locale>` で持つ。
    Locale {
        Ja => "ja",
        En => "en",
    }
}

/// 凪メーターの時間帯：始業と終業（`HH:MM`）。始業は終業より前でなければならない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeterHours {
    pub start: DayBoundary,
    pub end: DayBoundary,
}

impl MeterHours {
    /// 始業が終業より前なら作る。
    pub fn new(start: DayBoundary, end: DayBoundary) -> Option<Self> {
        (minutes(start) < minutes(end)).then_some(Self { start, end })
    }

    /// `"HH:MM-HH:MM"` から読む。
    pub fn parse(s: &str) -> Option<Self> {
        let (start, end) = s.split_once('-')?;
        Self::new(start.parse().ok()?, end.parse().ok()?)
    }

    /// 設定に保存する文字列（`"HH:MM-HH:MM"`）。
    pub fn to_value(self) -> String {
        format!("{}-{}", self.start, self.end)
    }
}

impl Default for MeterHours {
    fn default() -> Self {
        Self::parse(DEFAULT_METER_HOURS).expect("the default hours are valid")
    }
}

fn minutes(t: DayBoundary) -> u32 {
    u32::from(t.hour()) * 60 + u32::from(t.minute())
}

/// 画面に出す設定（既定値を当てた後のもの）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// 日付の切り替わり時刻（既定 4:00）。
    pub day_boundary: DayBoundary,
    /// 凪メーターの時間帯（既定 9:00〜19:00）。
    pub meter_hours: MeterHours,
    /// 画面の言語。設定が無ければ `None`（OS の言語に合わせる）。
    pub locale: Option<Locale>,
    /// 更新の自動確認（既定 `true`）。
    pub update_check: bool,
    pub theme: Theme,
    /// フォントサイズ（px。13〜19、既定 15）。
    pub font_size: u8,
    pub note_font: NoteFont,
    pub layout: Layout,
    /// 持ち越し見直しの回数（1 以上、既定 3）。
    pub carryover_review_threshold: u32,
    /// サイドバーの開閉（広いウィンドウの整理モード。既定は開く）。
    pub sidebar_open: bool,
    /// ショートカットの割当：操作の名前から割当の文字列（`Mod+Shift+L` など）へ。利用者が初期割当から
    /// 変えたものだけを持つ。名前と文字列の決まりは画面側が持つ（コアは形だけを守る。`valid_shortcuts`）。
    pub shortcuts: BTreeMap<String, String>,
    /// Vim モード（既定 `false`）。
    pub vim_mode: bool,
    /// Vim モードの挿入モードに入ったとき、直前の入力状態に戻す（既定 `true`）。
    pub vim_ime_restore: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            day_boundary: DayBoundary::DEFAULT,
            meter_hours: MeterHours::default(),
            locale: None,
            update_check: true,
            theme: Theme::default(),
            font_size: DEFAULT_FONT_SIZE,
            note_font: NoteFont::default(),
            layout: Layout::default(),
            carryover_review_threshold: carryover::DEFAULT_REVIEW_THRESHOLD as u32,
            sidebar_open: true,
            shortcuts: BTreeMap::new(),
            vim_mode: false,
            vim_ime_restore: true,
        }
    }
}

/// 変える設定：変える欄だけ `Some`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsPatch {
    pub day_boundary: Option<DayBoundary>,
    /// 始業と終業。始業が終業より前でなければ断る。
    pub meter_hours: Option<(DayBoundary, DayBoundary)>,
    pub locale: Option<Locale>,
    pub update_check: Option<bool>,
    pub theme: Option<Theme>,
    /// 13〜19 以外は断る。
    pub font_size: Option<u8>,
    pub note_font: Option<NoteFont>,
    pub layout: Option<Layout>,
    /// 1 以上でなければ断る。
    pub carryover_review_threshold: Option<u32>,
    pub sidebar_open: Option<bool>,
    /// 割当の全体（欄ごと置き換える）。形に合わなければ断る（`valid_shortcuts`）。
    pub shortcuts: Option<BTreeMap<String, String>>,
    pub vim_mode: Option<bool>,
    pub vim_ime_restore: Option<bool>,
}

/// 設定を変えられなかった理由。
#[derive(Debug)]
pub enum SettingsError {
    /// 値が誤っている。何も書いていない。
    Invalid(InvalidSetting),
    /// 読み書きに失敗した。
    Storage(StorageError),
}

/// 誤っている設定の欄。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidSetting {
    /// 凪メーターの時間帯（始業が終業より前でない）。
    MeterHours,
    /// フォントサイズ（13〜19 でない）。
    FontSize,
    /// 持ち越し見直しの回数（1 以上でない）。
    ReviewThreshold,
    /// ショートカットの割当（形に合わない）。
    Shortcuts,
}

/// 割当の件数の上限（壊れた値や誤った呼び出しで、設定が際限なく大きくならないように）。
pub const MAX_SHORTCUTS: usize = 64;
/// 割当の名前・文字列の長さの上限（文字数）。
pub const MAX_SHORTCUT_TEXT: usize = 64;

/// 割当の形が守られているか：件数が [`MAX_SHORTCUTS`] 以下で、名前と文字列がどちらも空でなく、
/// [`MAX_SHORTCUT_TEXT`] 文字以下で、制御文字を含まない。どの名前がどの文字列を取れるかは、キーを
/// 受ける画面側が決める（コアは受けないので、2つの決まりがずれないよう形だけを守る）。
pub fn valid_shortcuts(map: &BTreeMap<String, String>) -> bool {
    let ok = |s: &String| {
        !s.is_empty() && s.chars().count() <= MAX_SHORTCUT_TEXT && !s.chars().any(char::is_control)
    };
    map.len() <= MAX_SHORTCUTS && map.iter().all(|(k, v)| ok(k) && ok(v))
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(field) => write!(f, "invalid value for the {field:?} setting"),
            // 包むだけなので、中身をそのまま見せる（透過）
            Self::Storage(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid(_) => None,
            Self::Storage(e) => e.source(),
        }
    }
}

impl From<StorageError> for SettingsError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}

impl From<rusqlite::Error> for SettingsError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Storage(e.into())
    }
}

/// キー `key` の値を JSON の `T` として読む。未設定と、読めない値は `None`。
fn read<T: DeserializeOwned>(conn: &Connection, key: &str) -> Result<Option<T>, StorageError> {
    Ok(setting::get(conn, key)?.and_then(|v| serde_json::from_str(&v).ok()))
}

/// キー `key` の文字列の値を `parse` で読む。未設定と、読めない値は `None`。
fn read_enum<T>(
    conn: &Connection,
    key: &str,
    parse: fn(&str) -> Option<T>,
) -> Result<Option<T>, StorageError> {
    Ok(read::<String>(conn, key)?.and_then(|s| parse(&s)))
}

fn write<T: Serialize>(conn: &Connection, key: &str, value: &T) -> Result<(), StorageError> {
    // 文字列・数・真偽値の JSON は作れないことが無い
    let json = serde_json::to_string(value).expect("a plain value is serializable");
    setting::set(conn, key, &json)
}

/// 設定を読む。未設定と形に合わない値は既定値にする。
pub fn load(conn: &Connection) -> Result<Settings, StorageError> {
    let d = Settings::default();
    Ok(Settings {
        day_boundary: day::day_boundary(conn)?,
        meter_hours: read_enum(conn, METER_HOURS_KEY, MeterHours::parse)?.unwrap_or(d.meter_hours),
        locale: read_enum(conn, LOCALE_KEY, Locale::parse)?,
        update_check: read(conn, UPDATE_CHECK_KEY)?.unwrap_or(d.update_check),
        theme: read_enum(conn, THEME_KEY, Theme::parse)?.unwrap_or(d.theme),
        font_size: read::<u8>(conn, FONT_SIZE_KEY)?
            .filter(|n| FONT_SIZE_RANGE.contains(n))
            .unwrap_or(d.font_size),
        note_font: read_enum(conn, NOTE_FONT_KEY, NoteFont::parse)?.unwrap_or(d.note_font),
        layout: read_enum(conn, LAYOUT_KEY, Layout::parse)?.unwrap_or(d.layout),
        carryover_review_threshold: carryover::review_threshold(conn)? as u32,
        sidebar_open: read(conn, SIDEBAR_KEY)?.unwrap_or(d.sidebar_open),
        shortcuts: read::<BTreeMap<String, String>>(conn, SHORTCUTS_KEY)?
            .filter(valid_shortcuts)
            .unwrap_or(d.shortcuts),
        vim_mode: read(conn, VIM_MODE_KEY)?.unwrap_or(d.vim_mode),
        vim_ime_restore: read(conn, VIM_IME_RESTORE_KEY)?.unwrap_or(d.vim_ime_restore),
    })
}

/// `patch` の欄を書く。誤った値が1つでもあれば、何も書かずに断る。書いたら、書く前と変わったかを返す。
pub fn save(conn: &Connection, patch: &SettingsPatch) -> Result<bool, SettingsError> {
    let hours = match patch.meter_hours {
        Some((start, end)) => Some(
            MeterHours::new(start, end)
                .ok_or(SettingsError::Invalid(InvalidSetting::MeterHours))?,
        ),
        None => None,
    };
    if patch
        .font_size
        .is_some_and(|n| !FONT_SIZE_RANGE.contains(&n))
    {
        return Err(SettingsError::Invalid(InvalidSetting::FontSize));
    }
    if patch.carryover_review_threshold == Some(0) {
        return Err(SettingsError::Invalid(InvalidSetting::ReviewThreshold));
    }
    if patch
        .shortcuts
        .as_ref()
        .is_some_and(|m| !valid_shortcuts(m))
    {
        return Err(SettingsError::Invalid(InvalidSetting::Shortcuts));
    }
    in_transaction(conn, || {
        let before = load(conn)?;
        if let Some(v) = patch.day_boundary {
            write(conn, DAY_BOUNDARY_KEY, &v.to_string())?;
        }
        if let Some(v) = hours {
            write(conn, METER_HOURS_KEY, &v.to_value())?;
        }
        if let Some(v) = patch.locale {
            write(conn, LOCALE_KEY, &v.as_str())?;
        }
        if let Some(v) = patch.update_check {
            write(conn, UPDATE_CHECK_KEY, &v)?;
        }
        if let Some(v) = patch.theme {
            write(conn, THEME_KEY, &v.as_str())?;
        }
        if let Some(v) = patch.font_size {
            write(conn, FONT_SIZE_KEY, &v)?;
        }
        if let Some(v) = patch.note_font {
            write(conn, NOTE_FONT_KEY, &v.as_str())?;
        }
        if let Some(v) = patch.layout {
            write(conn, LAYOUT_KEY, &v.as_str())?;
        }
        if let Some(v) = patch.carryover_review_threshold {
            write(conn, REVIEW_THRESHOLD_KEY, &v)?;
        }
        if let Some(v) = patch.sidebar_open {
            write(conn, SIDEBAR_KEY, &v)?;
        }
        if let Some(v) = &patch.shortcuts {
            write(conn, SHORTCUTS_KEY, v)?;
        }
        if let Some(v) = patch.vim_mode {
            write(conn, VIM_MODE_KEY, &v)?;
        }
        if let Some(v) = patch.vim_ime_restore {
            write(conn, VIM_IME_RESTORE_KEY, &v)?;
        }
        Ok::<_, SettingsError>(load(conn)? != before)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::table::test_support::open;

    fn hhmm(s: &str) -> DayBoundary {
        s.parse().unwrap()
    }

    fn shortcuts(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn unset_settings_are_the_defaults() {
        let (db, _dir) = open();
        let s = load(db.connection()).unwrap();
        assert_eq!(s, Settings::default());
        assert_eq!(s.day_boundary, hhmm("04:00"));
        assert_eq!(s.meter_hours.to_value(), "09:00-19:00");
        assert_eq!(s.locale, None);
        assert!(s.update_check && s.sidebar_open);
        assert_eq!(
            (s.theme, s.note_font, s.layout),
            (Theme::System, NoteFont::Body, Layout::A)
        );
        assert_eq!((s.font_size, s.carryover_review_threshold), (15, 3));
    }

    #[test]
    fn saved_settings_are_read_back_and_only_changes_are_reported() {
        let (db, _dir) = open();
        let conn = db.connection();
        let patch = SettingsPatch {
            day_boundary: Some(hhmm("05:30")),
            meter_hours: Some((hhmm("08:30"), hhmm("18:00"))),
            locale: Some(Locale::Ja),
            update_check: Some(false),
            theme: Some(Theme::Dark),
            font_size: Some(17),
            note_font: Some(NoteFont::Mono),
            layout: Some(Layout::C),
            carryover_review_threshold: Some(5),
            sidebar_open: Some(false),
            shortcuts: Some(shortcuts(&[("palette", "Mod+P"), ("prevDay", "Alt+[")])),
            vim_mode: Some(true),
            vim_ime_restore: Some(false),
        };
        assert!(save(conn, &patch).unwrap());
        let s = load(conn).unwrap();
        assert_eq!(s.day_boundary, hhmm("05:30"));
        assert_eq!(s.meter_hours.to_value(), "08:30-18:00");
        assert_eq!(s.locale, Some(Locale::Ja));
        assert!(!s.update_check && !s.sidebar_open);
        assert_eq!(
            (s.theme, s.note_font, s.layout),
            (Theme::Dark, NoteFont::Mono, Layout::C)
        );
        assert_eq!((s.font_size, s.carryover_review_threshold), (17, 5));
        assert_eq!(
            s.shortcuts,
            shortcuts(&[("palette", "Mod+P"), ("prevDay", "Alt+[")])
        );
        assert!(s.vim_mode && !s.vim_ime_restore);
        assert_eq!(
            setting::get(conn, SHORTCUTS_KEY).unwrap().as_deref(),
            Some(r#"{"palette":"Mod+P","prevDay":"Alt+["}"#)
        );
        // 保存した形（データモデル設計書 3章 setting）
        assert_eq!(
            setting::get(conn, METER_HOURS_KEY).unwrap().as_deref(),
            Some(r#""08:30-18:00""#)
        );
        assert_eq!(
            setting::get(conn, DAY_BOUNDARY_KEY).unwrap().as_deref(),
            Some(r#""05:30""#)
        );
        // 同じ値を書いても、変わらない。欄の無い patch も何も変えない
        assert!(!save(conn, &patch).unwrap());
        assert!(!save(conn, &SettingsPatch::default()).unwrap());
        // 一部の欄だけ変えても、ほかは残る
        let one = SettingsPatch {
            theme: Some(Theme::Light),
            ..SettingsPatch::default()
        };
        assert!(save(conn, &one).unwrap());
        let s = load(conn).unwrap();
        assert_eq!(
            (s.theme, s.font_size, s.layout),
            (Theme::Light, 17, Layout::C)
        );
    }

    #[test]
    fn wrong_values_are_refused_without_writing_anything() {
        let (db, _dir) = open();
        let conn = db.connection();
        let bad = |patch: SettingsPatch, field| {
            // ほかの正しい欄も書かない
            let patch = SettingsPatch {
                theme: Some(Theme::Dark),
                ..patch
            };
            assert!(matches!(
                save(conn, &patch),
                Err(SettingsError::Invalid(f)) if f == field
            ));
            assert_eq!(load(conn).unwrap(), Settings::default());
        };
        bad(
            SettingsPatch {
                meter_hours: Some((hhmm("19:00"), hhmm("09:00"))),
                ..SettingsPatch::default()
            },
            InvalidSetting::MeterHours,
        );
        bad(
            SettingsPatch {
                meter_hours: Some((hhmm("09:00"), hhmm("09:00"))),
                ..SettingsPatch::default()
            },
            InvalidSetting::MeterHours,
        );
        for n in [12, 20, 0, 255] {
            bad(
                SettingsPatch {
                    font_size: Some(n),
                    ..SettingsPatch::default()
                },
                InvalidSetting::FontSize,
            );
        }
        bad(
            SettingsPatch {
                carryover_review_threshold: Some(0),
                ..SettingsPatch::default()
            },
            InvalidSetting::ReviewThreshold,
        );
        // 割当の形：空の名前・文字列、長すぎる文字列、制御文字、多すぎる件数
        let too_many: Vec<(String, String)> = (0..=MAX_SHORTCUTS)
            .map(|i| (format!("a{i}"), "Mod+P".to_string()))
            .collect();
        let long = "x".repeat(MAX_SHORTCUT_TEXT + 1);
        for map in [
            shortcuts(&[("", "Mod+P")]),
            shortcuts(&[("palette", "")]),
            shortcuts(&[("palette", &long)]),
            shortcuts(&[("palette", "Mod+\n")]),
            too_many.into_iter().collect(),
        ] {
            bad(
                SettingsPatch {
                    shortcuts: Some(map),
                    ..SettingsPatch::default()
                },
                InvalidSetting::Shortcuts,
            );
        }
    }

    #[test]
    fn an_empty_shortcut_map_clears_the_overrides() {
        let (db, _dir) = open();
        let conn = db.connection();
        let set = |map| SettingsPatch {
            shortcuts: Some(map),
            ..SettingsPatch::default()
        };
        assert!(save(conn, &set(shortcuts(&[("today", "Mod+Y")]))).unwrap());
        // 割当は欄ごと置き換える（変わらない欄の割当は残らない）
        assert!(save(conn, &set(shortcuts(&[("palette", "Mod+P")]))).unwrap());
        assert_eq!(
            load(conn).unwrap().shortcuts,
            shortcuts(&[("palette", "Mod+P")])
        );
        assert!(save(conn, &set(BTreeMap::new())).unwrap());
        assert!(load(conn).unwrap().shortcuts.is_empty());
    }

    #[test]
    fn wrong_stored_values_fall_back_to_the_defaults() {
        let (db, _dir) = open();
        let conn = db.connection();
        for (key, bad) in [
            (THEME_KEY, r#""sepia""#),
            (THEME_KEY, "3"),
            (FONT_SIZE_KEY, "12"),
            (FONT_SIZE_KEY, "300"),
            (FONT_SIZE_KEY, r#""15""#),
            (NOTE_FONT_KEY, r#""serif""#),
            (LAYOUT_KEY, r#""D""#),
            (LOCALE_KEY, r#""fr""#),
            (UPDATE_CHECK_KEY, r#""yes""#),
            (METER_HOURS_KEY, r#""19:00-09:00""#),
            (METER_HOURS_KEY, r#""9:00-19:00""#),
            (METER_HOURS_KEY, "900"),
            (SIDEBAR_KEY, "1"),
            (REVIEW_THRESHOLD_KEY, "0"),
            (SHORTCUTS_KEY, r#"["Mod+P"]"#),
            (SHORTCUTS_KEY, r#"{"palette":3}"#),
            (SHORTCUTS_KEY, r#"{"palette":""}"#),
            (VIM_MODE_KEY, r#""on""#),
            (VIM_IME_RESTORE_KEY, "0"),
        ] {
            setting::set(conn, key, bad).unwrap();
        }
        assert_eq!(load(conn).unwrap(), Settings::default());
    }

    #[test]
    fn meter_hours_need_the_start_before_the_end() {
        assert!(MeterHours::parse("00:00-23:59").is_some());
        for bad in [
            "09:00-09:00",
            "10:00-09:00",
            "09:00",
            "09:00-",
            "-19:00",
            "09:00-24:00",
            "",
        ] {
            assert_eq!(MeterHours::parse(bad), None, "{bad}");
        }
    }
}
