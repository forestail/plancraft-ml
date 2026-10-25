//! Interface language: English (the default, used by tests and the MCP server) or Russian.
//!
//! `tr("English text")` returns the translation of a fixed string. `tr_dyn(&format!(...))`
//! translates a formatted message: table keys may contain `{}` placeholders that match any
//! text, and the matched parts are carried over in order into the translation.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// Interface language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    #[default]
    En,
    Ru,
    Ja,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::Ru, Lang::En, Lang::Ja];

    /// The language's own name, for a language picker.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ru => "Русский",
            Lang::Ja => "日本語",
        }
    }

    /// Russian when the system locale (LANG / LC_ALL / LC_MESSAGES) is Russian, otherwise English.
    pub fn from_env() -> Lang {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"] {
            if let Ok(v) = std::env::var(var) {
                let v = v.to_ascii_lowercase();
                if v.is_empty() {
                    continue;
                }
                return if v.starts_with("ru") {
                    Lang::Ru
                } else if v.starts_with("ja") {
                    Lang::Ja
                } else {
                    Lang::En
                };
            }
        }
        Lang::En
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(lang: Lang) {
    LANG.store(
        match lang {
            Lang::En => 0,
            Lang::Ru => 1,
            Lang::Ja => 2,
        },
        Ordering::Relaxed,
    );
}

pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Ru,
        2 => Lang::Ja,
        _ => Lang::En,
    }
}

pub fn is_ru() -> bool {
    lang() == Lang::Ru
}

pub fn is_ja() -> bool {
    lang() == Lang::Ja
}

static EXACT: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| crate::i18n_ru::RU.iter().filter(|(k, _)| !k.contains("{}")).copied().collect());

struct Template {
    /// Literal pieces of the key around the placeholders (`parts.len() == holes + 1`).
    parts: Vec<&'static str>,
    out: Vec<&'static str>,
}

static TEMPLATES: LazyLock<Vec<Template>> = LazyLock::new(|| {
    let mut v: Vec<Template> = crate::i18n_ru::RU
        .iter()
        .filter(|(k, _)| k.contains("{}"))
        .map(|(k, t)| Template { parts: k.split("{}").collect(), out: t.split("{}").collect() })
        .collect();
    // Most specific (longest literal text) first.
    v.sort_by_key(|t| std::cmp::Reverse(t.parts.iter().map(|p| p.len()).sum::<usize>()));
    v
});

/// Translates a fixed interface string.
pub fn tr(s: &'static str) -> &'static str {
    if lang() == Lang::Ja {
        return crate::i18n_ja::translate(s).unwrap_or(s);
    }
    if !is_ru() {
        return s;
    }
    EXACT.get(s).copied().unwrap_or(s)
}

static REVERSE: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for (k, v) in crate::i18n_ru::RU.iter().filter(|(k, _)| !k.contains("{}")) {
        m.entry(*v).or_insert(*k);
    }
    m
});

/// The English original of a translated interface string (for choice lists whose
/// selected text is matched in code); unknown text is returned unchanged.
pub fn untr(s: &str) -> &str {
    if is_ja() {
        return crate::i18n_ja::untranslate(s);
    }
    REVERSE.get(s).copied().unwrap_or(s)
}

/// Translates a formatted interface message.
pub fn tr_dyn(s: &str) -> String {
    if lang() == Lang::Ja {
        return crate::i18n_ja::translate_dynamic(s)
            .or_else(|| crate::i18n_ja::translate(s).map(str::to_string))
            .unwrap_or_else(|| s.to_string());
    }
    if !is_ru() {
        return s.to_string();
    }
    if let Some(t) = EXACT.get(s) {
        return (*t).to_string();
    }
    for t in TEMPLATES.iter() {
        if let Some(caps) = match_template(&t.parts, s) {
            let mut out = String::new();
            for (i, piece) in t.out.iter().enumerate() {
                out.push_str(piece);
                if i + 1 < t.out.len() {
                    // Captured parts are translated too, e.g. "Delete {}" with a view name.
                    let cap = caps.get(i).copied().unwrap_or("");
                    match EXACT.get(cap) {
                        Some(c) => out.push_str(c),
                        None => out.push_str(cap),
                    }
                }
            }
            return out;
        }
    }
    s.to_string()
}

/// Matches `s` against literal `parts` separated by wildcards; returns the wildcard captures.
pub(crate) fn match_template<'a>(parts: &[&str], s: &'a str) -> Option<Vec<&'a str>> {
    let (first, rest) = parts.split_first()?;
    let mut tail = s.strip_prefix(first)?;
    let mut caps = Vec::with_capacity(rest.len());
    for (i, part) in rest.iter().enumerate() {
        let last = i + 1 == rest.len();
        if last {
            let cap = tail.strip_suffix(part)?;
            if cap.is_empty() && !part.is_empty() && !first.is_empty() {
                // Allow empty captures; nothing special to do.
            }
            caps.push(cap);
            tail = "";
        } else {
            if part.is_empty() {
                return None;
            }
            let pos = tail.find(part)?;
            caps.push(tail.get(..pos)?);
            tail = tail.get(pos + part.len()..)?;
        }
    }
    if rest.is_empty() && !tail.is_empty() {
        return None;
    }
    Some(caps)
}

/// Short weekday name, Monday = 0.
pub fn weekday_short(wd: chrono::Weekday) -> &'static str {
    use chrono::Weekday::*;
    if is_ja() {
        match wd {
            Mon => "月",
            Tue => "火",
            Wed => "水",
            Thu => "木",
            Fri => "金",
            Sat => "土",
            Sun => "日",
        }
    } else if is_ru() {
        match wd {
            Mon => "Пн",
            Tue => "Вт",
            Wed => "Ср",
            Thu => "Чт",
            Fri => "Пт",
            Sat => "Сб",
            Sun => "Вс",
        }
    } else {
        match wd {
            Mon => "Mon",
            Tue => "Tue",
            Wed => "Wed",
            Thu => "Thu",
            Fri => "Fri",
            Sat => "Sat",
            Sun => "Sun",
        }
    }
}

/// One-letter weekday for timescales.
pub fn weekday_letter(wd: chrono::Weekday) -> &'static str {
    use chrono::Weekday::*;
    if is_ja() {
        match wd {
            Mon => "月",
            Tue => "火",
            Wed => "水",
            Thu => "木",
            Fri => "金",
            Sat => "土",
            Sun => "日",
        }
    } else if is_ru() {
        match wd {
            Mon => "П",
            Tue => "В",
            Wed => "С",
            Thu => "Ч",
            Fri => "П",
            Sat => "С",
            Sun => "В",
        }
    } else {
        match wd {
            Mon => "M",
            Tue => "T",
            Wed => "W",
            Thu => "T",
            Fri => "F",
            Sat => "S",
            Sun => "S",
        }
    }
}

/// Short month name, January = 1.
pub fn month_short(m: u32) -> &'static str {
    const EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    const RU: [&str; 12] = ["янв", "фев", "мар", "апр", "май", "июн", "июл", "авг", "сен", "окт", "ноя", "дек"];
    const JA: [&str; 12] = ["1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月"];
    let i = (m.clamp(1, 12) - 1) as usize;
    if is_ja() {
        JA.get(i).copied().unwrap_or("")
    } else if is_ru() {
        RU.get(i).copied().unwrap_or("")
    } else {
        EN.get(i).copied().unwrap_or("")
    }
}

/// Full month name (nominative), January = 1.
pub fn month_long(m: u32) -> &'static str {
    const EN: [&str; 12] =
        ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    const RU: [&str; 12] =
        ["Январь", "Февраль", "Март", "Апрель", "Май", "Июнь", "Июль", "Август", "Сентябрь", "Октябрь", "Ноябрь", "Декабрь"];
    const JA: [&str; 12] = ["1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月"];
    let i = (m.clamp(1, 12) - 1) as usize;
    if is_ja() {
        JA.get(i).copied().unwrap_or("")
    } else if is_ru() {
        RU.get(i).copied().unwrap_or("")
    } else {
        EN.get(i).copied().unwrap_or("")
    }
}

/// Parses a decimal number written with either a point or a comma ("1,5" or "1.5").
pub fn parse_f64(text: &str) -> Option<f64> {
    let t: String = text.trim().chars().filter(|c| !c.is_whitespace() && *c != '\u{a0}').collect();
    let t = if t.contains(',') && !t.contains('.') { t.replace(',', ".") } else { t };
    let v: f64 = t.parse().ok()?;
    v.is_finite().then_some(v)
}

/// Parses a date in ISO (`2026-10-19`), Russian (`19.10.2026`, `19.10.26`) or US (`10/19/2026`) form.
pub fn parse_date(text: &str) -> Option<chrono::NaiveDate> {
    use chrono::NaiveDate;
    let t = text.trim();
    // Drop a leading weekday such as "Пн " or "Mon ".
    let t = match t.split_once(' ') {
        Some((head, rest)) if head.chars().all(|c| c.is_alphabetic()) => rest.trim(),
        _ => t,
    };
    // Ignore a trailing time.
    let t = t.split_whitespace().next().unwrap_or("");
    if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return Some(d);
    }
    if t.contains('.') {
        let mut it = t.split('.');
        let d: u32 = it.next()?.parse().ok()?;
        let m: u32 = it.next()?.parse().ok()?;
        let y: i32 = it.next()?.parse().ok()?;
        let y = if y < 100 { 2000 + y } else { y };
        return NaiveDate::from_ymd_opt(y, m, d);
    }
    if t.contains('/') {
        let mut it = t.split('/');
        let a = it.next()?;
        let b: u32 = it.next()?.parse().ok()?;
        let c: u32 = it.next()?.parse().ok()?;
        if a.len() == 4 {
            let y: i32 = a.parse().ok()?;
            return NaiveDate::from_ymd_opt(y, b, c);
        }
        let m: u32 = a.parse().ok()?;
        let y = if c < 100 { 2000 + c as i32 } else { c as i32 };
        return NaiveDate::from_ymd_opt(y, m, b);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_and_dates() {
        assert_eq!(match_template(&["Week of ", ""], "Week of Oct 5"), Some(vec!["Oct 5"]));
        assert_eq!(match_template(&["", " of ", "!"], "a of b!"), Some(vec!["a", "b"]));
        assert_eq!(match_template(&["x"], "y"), None);
        assert_eq!(parse_date("19.10.2026"), chrono::NaiveDate::from_ymd_opt(2026, 10, 19));
        assert_eq!(parse_date("Пн 19.10.26"), chrono::NaiveDate::from_ymd_opt(2026, 10, 19));
        assert_eq!(parse_date("2026-10-19"), chrono::NaiveDate::from_ymd_opt(2026, 10, 19));
        assert_eq!(parse_f64("1,5"), Some(1.5));
    }
}
