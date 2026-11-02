// SPDX-License-Identifier: MIT OR Apache-2.0

//! Text parsing exposed to Lua as `parse.*` (DESIGN §4.5).
//!
//! Every function trims surrounding whitespace and returns `None` when the text is not
//! parseable. Results are always finite. The Unicode minus sign `−` (U+2212) is accepted
//! wherever `-` is.

use super::fmt::{
    DAYS_LONG, DAYS_SHORT, MONTHS_LONG, MONTHS_SHORT, days_from_civil, days_in_month, weekday,
};

const UNICODE_MINUS: char = '\u{2212}';

fn is_minus(c: char) -> bool {
    c == '-' || c == UNICODE_MINUS
}

/// Splits an optional leading sign off `s`: `Some(true)` for minus, `Some(false)` for plus.
fn strip_sign(s: &str) -> (Option<bool>, &str) {
    match s.chars().next() {
        Some(c) if is_minus(c) => (Some(true), &s[c.len_utf8()..]),
        Some('+') => (Some(false), &s[1..]),
        _ => (None, s),
    }
}

/// A plain number: `"1,234.5"`, `"1_000"`, `"-.5"`, `"1e3"`.
///
/// `,` and `_` are digit-group separators: each must sit between two digits of the integer
/// part (group sizes are not checked). Infinity, NaN, empty text and out-of-range values
/// (`"1e999"`) are rejected.
pub fn number(text: &str) -> Option<f64> {
    let t = text.trim();
    let chars: Vec<char> = t
        .chars()
        .map(|c| if c == UNICODE_MINUS { '-' } else { c })
        .collect();
    if chars.is_empty() {
        return None;
    }
    let mut cleaned = String::with_capacity(chars.len());
    let mut seen_dot = false;
    let mut seen_exp = false;
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '0'..='9' | '+' | '-' => cleaned.push(c),
            ',' | '_' => {
                let prev_digit = i
                    .checked_sub(1)
                    .and_then(|j| chars.get(j))
                    .is_some_and(char::is_ascii_digit);
                let next_digit = chars.get(i + 1).is_some_and(char::is_ascii_digit);
                if !prev_digit || !next_digit || seen_dot || seen_exp {
                    return None;
                }
            }
            '.' => {
                if seen_dot || seen_exp {
                    return None;
                }
                seen_dot = true;
                cleaned.push('.');
            }
            'e' | 'E' => {
                if seen_exp {
                    return None;
                }
                seen_exp = true;
                cleaned.push('e');
            }
            _ => return None,
        }
    }
    // Sign placement, digits around the exponent, etc. are validated by the std parser.
    let v: f64 = cleaned.parse().ok()?;
    v.is_finite().then_some(v)
}

/// Hemisphere letter: `Some(true)` if it negates (S/W).
fn hemisphere(c: char) -> Option<bool> {
    match c.to_ascii_uppercase() {
        'N' | 'E' => Some(false),
        'S' | 'W' => Some(true),
        _ => None,
    }
}

/// Degrees, minutes and seconds, or decimal degrees.
///
/// Components are numbers separated by whitespace and/or the markers `°` (also `º`),
/// `′`/`'` (minutes) and `″`/`"`/`''` (seconds). A marker fixes the component it follows;
/// unmarked components fill degrees, minutes, seconds in order. Components must appear in
/// that order, only the last may have a fraction, and minutes and seconds must be < 60.
/// A hemisphere letter N/S/E/W (any case) may appear first or last; S and W negate. A
/// hemisphere letter combined with a minus sign is rejected. Degrees are not range-checked.
pub fn dms(text: &str) -> Option<f64> {
    let mut body = text.trim();
    let mut hemi: Option<bool> = None;
    if let Some(c) = body.chars().next()
        && let Some(neg) = hemisphere(c)
    {
        hemi = Some(neg);
        body = &body[c.len_utf8()..];
    }
    if let Some(c) = body.chars().next_back()
        && let Some(neg) = hemisphere(c)
    {
        if hemi.is_some() {
            return None;
        }
        hemi = Some(neg);
        body = &body[..body.len() - c.len_utf8()];
    }
    let (sign, body) = strip_sign(body.trim());
    if sign == Some(true) && hemi.is_some() {
        return None;
    }

    let chars: Vec<char> = body.chars().collect();
    let skip_ws = |i: &mut usize| {
        while chars.get(*i).is_some_and(|c| c.is_whitespace()) {
            *i += 1;
        }
    };
    let mut parts: [Option<f64>; 3] = [None; 3];
    let mut next_slot = 0usize;
    let mut prev_had_fraction = false;
    let mut i = 0usize;
    skip_ws(&mut i);
    while i < chars.len() {
        let start = i;
        while chars
            .get(i)
            .is_some_and(|c| c.is_ascii_digit() || *c == '.')
        {
            i += 1;
        }
        let token: String = chars.get(start..i)?.iter().collect();
        if !token.chars().any(|c| c.is_ascii_digit()) {
            return None;
        }
        let value: f64 = token.parse().ok()?;
        skip_ws(&mut i);
        let marker = match chars.get(i) {
            Some('°' | 'º') => {
                i += 1;
                Some(0)
            }
            Some('′' | '\'') => {
                i += 1;
                if chars.get(i) == Some(&'\'') {
                    i += 1;
                    Some(2)
                } else {
                    Some(1)
                }
            }
            Some('″' | '"') => {
                i += 1;
                Some(2)
            }
            _ => None,
        };
        skip_ws(&mut i);
        let slot = marker.unwrap_or(next_slot);
        if slot < next_slot || prev_had_fraction {
            return None;
        }
        *parts.get_mut(slot)? = Some(value);
        next_slot = slot + 1;
        prev_had_fraction = token.contains('.');
    }
    if parts.iter().all(Option::is_none) {
        return None;
    }
    let [d, m, s] = parts.map(|p| p.unwrap_or(0.0));
    if m >= 60.0 || s >= 60.0 {
        return None;
    }
    let total = d + m / 60.0 + s / 3600.0;
    if !total.is_finite() {
        return None;
    }
    let negative = sign == Some(true) || hemi == Some(true);
    Some(if negative { -total } else { total })
}

/// Reads between `min` and `max` ASCII digits at `*i`.
fn read_digits(text: &[char], i: &mut usize, min: usize, max: usize) -> Option<i64> {
    let mut n: i64 = 0;
    let mut count = 0usize;
    while count < max
        && let Some(d) = text.get(*i).and_then(|c| c.to_digit(10))
    {
        n = n.checked_mul(10)?.checked_add(i64::from(d))?;
        *i += 1;
        count += 1;
    }
    (count >= min).then_some(n)
}

/// Case-insensitively matches one of `names` (tried in order) at `*i`; returns its index.
fn read_name(text: &[char], i: &mut usize, names: &[&[&str]]) -> Option<usize> {
    for table in names {
        for (idx, name) in table.iter().enumerate() {
            let len = name.chars().count();
            let Some(candidate) = text.get(*i..*i + len) else {
                continue;
            };
            if candidate
                .iter()
                .zip(name.chars())
                .all(|(a, b)| a.to_lowercase().eq(b.to_lowercase()))
            {
                *i += len;
                return Some(idx);
            }
        }
    }
    None
}

fn is_numeric_spec(c: Option<&char>) -> bool {
    matches!(c, Some('Y' | 'y' | 'm' | 'd' | 'H' | 'M' | 'S'))
}

/// Inverse of [`super::fmt::date`]: days since 1970-01-01 UTC.
///
/// Supports `%Y %m %d %H %M %S %y %b %B %a %A %%`. Other characters in the pattern must match
/// exactly; unknown `%x` sequences match themselves literally (as `fmt.date` emits them),
/// except `%j`, which is unsupported and always fails. Details:
/// - `%Y`: optional `-`, then digits (at most 4 when directly followed by another numeric
///   field, otherwise unbounded).
/// - `%m %d %H %M %S`: one or two digits. `%y`: two digits, 69..=99 -> 19xx, else 20xx.
/// - `%b`/`%B`: full or abbreviated English month name, any case.
/// - `%a`/`%A`: full or abbreviated weekday name, any case; must agree with the date.
/// - Missing fields default to 1970, January, day 1, 00:00:00. Ranges are validated
///   (no Feb 30, no hour 24, no leap second).
pub fn date(text: &str, pattern: &str) -> Option<f64> {
    let text: Vec<char> = text.trim().chars().collect();
    let pat: Vec<char> = pattern.chars().collect();
    let (mut y, mut mo, mut d) = (1970i64, 1i64, 1i64);
    let (mut hh, mut mi, mut ss) = (0i64, 0i64, 0i64);
    let mut wanted_weekday: Option<usize> = None;
    let mut ti = 0usize;
    let mut pi = 0usize;
    let expect =
        |ti: &mut usize, c: char| -> Option<()> { (text.get(*ti) == Some(&c)).then(|| *ti += 1) };
    while let Some(&c) = pat.get(pi) {
        if c != '%' {
            expect(&mut ti, c)?;
            pi += 1;
            continue;
        }
        let spec = pat.get(pi + 1).copied();
        pi += 2;
        let next_numeric = pat.get(pi) == Some(&'%') && is_numeric_spec(pat.get(pi + 1));
        match spec {
            Some('Y') => {
                let neg = text.get(ti).is_some_and(|c| is_minus(*c));
                if neg {
                    ti += 1;
                }
                let max = if next_numeric { 4 } else { 12 };
                let n = read_digits(&text, &mut ti, 1, max)?;
                y = if neg { -n } else { n };
            }
            Some('y') => {
                let n = read_digits(&text, &mut ti, 2, 2)?;
                y = if n >= 69 { 1900 + n } else { 2000 + n };
            }
            Some('m') => mo = read_digits(&text, &mut ti, 1, 2)?,
            Some('d') => d = read_digits(&text, &mut ti, 1, 2)?,
            Some('H') => hh = read_digits(&text, &mut ti, 1, 2)?,
            Some('M') => mi = read_digits(&text, &mut ti, 1, 2)?,
            Some('S') => ss = read_digits(&text, &mut ti, 1, 2)?,
            Some('b' | 'B') => {
                let idx = read_name(&text, &mut ti, &[&MONTHS_LONG, &MONTHS_SHORT])?;
                mo = i64::try_from(idx).ok()? + 1;
            }
            Some('a' | 'A') => {
                wanted_weekday = Some(read_name(&text, &mut ti, &[&DAYS_LONG, &DAYS_SHORT])?);
            }
            Some('j') => return None,
            Some('%') | None => expect(&mut ti, '%')?,
            Some(other) => {
                expect(&mut ti, '%')?;
                expect(&mut ti, other)?;
            }
        }
    }
    if ti != text.len() {
        return None;
    }
    let mo = u32::try_from(mo).ok().filter(|m| (1..=12).contains(m))?;
    let d = u32::try_from(d)
        .ok()
        .filter(|d| *d >= 1 && *d <= days_in_month(y, mo))?;
    if !(0..24).contains(&hh) || !(0..60).contains(&mi) || !(0..60).contains(&ss) {
        return None;
    }
    let z = days_from_civil(y, mo, d);
    if wanted_weekday.is_some_and(|w| w != weekday(z)) {
        return None;
    }
    Some(z as f64 + (hh * 3600 + mi * 60 + ss) as f64 / 86_400.0)
}

/// Symbols stripped by [`currency`]; longer ones first so `US$` wins over `$`.
const CURRENCY_SYMBOLS: [&str; 26] = [
    "US$", "AU$", "NZ$", "CA$", "HK$", "NT$", "A$", "C$", "S$", "R$", "$", "€", "£", "¥", "₹", "₩",
    "₽", "₺", "₪", "₫", "₱", "₦", "₴", "₿", "¢", "kr",
];

fn is_code(bytes: &[u8]) -> bool {
    bytes.len() == 3 && bytes.iter().all(u8::is_ascii_uppercase)
}

fn strip_symbol_prefix(s: &str) -> Option<&str> {
    if let Some(rest) = CURRENCY_SYMBOLS.iter().find_map(|sym| s.strip_prefix(sym)) {
        return Some(rest);
    }
    let b = s.as_bytes();
    let code = b.get(..3).is_some_and(is_code) && !b.get(3).is_some_and(u8::is_ascii_alphabetic);
    code.then(|| &s[3..])
}

fn strip_symbol_suffix(s: &str) -> Option<&str> {
    if let Some(rest) = CURRENCY_SYMBOLS.iter().find_map(|sym| s.strip_suffix(sym)) {
        return Some(rest);
    }
    let b = s.as_bytes();
    let n = b.len();
    let code = n >= 3
        && b.get(n - 3..).is_some_and(is_code)
        && !(n > 3 && b.get(n - 4).is_some_and(u8::is_ascii_alphabetic));
    code.then(|| &s[..n - 3])
}

/// A money amount: `"$1,234.50"`, `"-$3"`, `"$-3"`, `"(12.50)"`, `"12.50 €"`, `"AUD 1,000"`.
///
/// At most one currency symbol or three-letter upper-case code, leading or trailing. At most
/// one sign: a leading `-`/`+` (before or after a leading symbol) or accounting parentheses.
/// Whitespace inside the amount is removed; the remaining digits follow [`number`]'s rules.
pub fn currency(text: &str) -> Option<f64> {
    let mut s = text.trim();
    let mut paren = false;
    if let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        paren = true;
        s = inner.trim();
    }
    let (mut sign, rest) = strip_sign(s);
    s = rest.trim_start();
    let mut had_symbol = false;
    if let Some(rest) = strip_symbol_prefix(s) {
        had_symbol = true;
        s = rest.trim_start();
        if sign.is_none() {
            let (inner_sign, rest) = strip_sign(s);
            sign = inner_sign;
            s = rest.trim_start();
        }
    }
    if !had_symbol && let Some(rest) = strip_symbol_suffix(s) {
        s = rest;
    }
    if paren && sign.is_some() {
        return None;
    }
    let amount: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !amount.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let v = number(&amount)?;
    Some(if paren || sign == Some(true) { -v } else { v })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stdlib::fmt::{self, DmsKind};
    use proptest::prelude::*;

    fn close(a: Option<f64>, b: f64) -> bool {
        a.is_some_and(|a| (a - b).abs() < 1e-9)
    }

    #[test]
    fn number_golden() {
        assert_eq!(number("1,234.5"), Some(1234.5));
        assert_eq!(number("1_000"), Some(1000.0));
        assert_eq!(number("-.5"), Some(-0.5));
        assert_eq!(number("+.5"), Some(0.5));
        assert_eq!(number("1e3"), Some(1000.0));
        assert_eq!(number("1E-3"), Some(0.001));
        assert_eq!(number("  42  "), Some(42.0));
        assert_eq!(number("5."), Some(5.0));
        assert_eq!(number("0"), Some(0.0));
        assert_eq!(number("-0").map(f64::is_sign_negative), Some(true));
        assert_eq!(number("\u{2212}3"), Some(-3.0));
        assert_eq!(number("1,234,567"), Some(1_234_567.0));
        assert_eq!(number("12,34"), Some(1234.0)); // group sizes not checked
        assert_eq!(number("1e308"), Some(1e308));
        assert_eq!(number("5e-324"), Some(5e-324));
        for bad in [
            "", "   ", "inf", "-inf", "Infinity", "nan", "NaN", "1e999", "-1e999", "abc", "1.2.3",
            "1e", "e5", "--5", "1-2", ".", "-", ",1", "1,", "1,,2", "1.2,3", "1e1,0", "_1", "1 2",
            "$5", "0x10", "1e3e3",
        ] {
            assert_eq!(number(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn dms_golden() {
        let canberra = 35.0 + 18.0 / 60.0 + 27.0 / 3600.0;
        assert!(close(dms("-35.3075"), -35.3075));
        assert!(close(dms("35°18'27\"S"), -canberra));
        assert!(close(dms("35 18 27 S"), -canberra));
        assert!(close(dms("S 35 18 27"), -canberra));
        assert!(close(dms("s35°18′27″"), -canberra));
        assert!(close(dms("35°18′27″N"), canberra));
        assert!(close(dms("-35 18 27"), -canberra));
        assert!(close(dms("\u{2212}35 18 27"), -canberra));
        assert!(close(dms("+35 18 27"), canberra));
        assert!(close(dms("35°18′27.5″ S"), -(canberra + 0.5 / 3600.0)));
        assert!(close(dms("35°18.5'S"), -(35.0 + 18.5 / 60.0)));
        assert!(close(dms("149°7′E"), 149.0 + 7.0 / 60.0));
        assert!(close(dms("149°7′w"), -(149.0 + 7.0 / 60.0)));
        assert!(close(dms("35°18''"), 35.0 + 18.0 / 3600.0));
        assert!(close(dms("35° 27\""), 35.0 + 27.0 / 3600.0));
        assert!(close(dms("18'"), 18.0 / 60.0));
        assert!(close(dms("35º18'"), 35.0 + 18.0 / 60.0));
        assert!(close(dms("0"), 0.0));
        assert!(close(dms("400"), 400.0));
        assert!(close(
            dms("1 59 59.999"),
            1.0 + 59.0 / 60.0 + 59.999 / 3600.0
        ));
        for bad in [
            "",
            "S",
            "N 35 18 27 S",
            "-35 18 27 S",
            "S -35",
            "35 60 0",
            "35 0 60",
            "35 61",
            "35.5 18",
            "35°18.5'27\"",
            "35 18 27 1",
            "35\"18'",
            "18' 35°",
            "abc",
            "35x",
            "35°°",
            "°35",
            "35 18 27 Q",
            "1.2.3",
            "--5",
            "N",
            "35d18m",
            "1e3",
        ] {
            assert_eq!(dms(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn date_golden() {
        let p = "%Y-%m-%d";
        assert_eq!(date("1970-01-01", p), Some(0.0));
        assert_eq!(date("1969-12-31", p), Some(-1.0));
        assert_eq!(date(" 2024-02-29 ", p), Some(19_782.0));
        assert_eq!(date("1900-01-01", p), Some(-25_567.0));
        assert_eq!(date("0000-01-01", p), Some(-719_528.0));
        assert_eq!(date("-0001-12-31", p), Some(-719_529.0));
        assert_eq!(
            date("2024-2-9", p),
            Some(days_from_civil(2024, 2, 9) as f64)
        );
        assert_eq!(date("10000-01-01", p), Some(2_932_897.0));
        assert_eq!(date("20240229", "%Y%m%d"), Some(19_782.0));
        assert_eq!(date("29 feb 2024", "%d %b %Y"), Some(19_782.0));
        assert_eq!(date("29 February 2024", "%d %b %Y"), Some(19_782.0));
        assert_eq!(date("29 FEBRUARY 2024", "%d %B %Y"), Some(19_782.0));
        assert_eq!(date("Thu 29 Feb 2024", "%a %d %b %Y"), Some(19_782.0));
        assert_eq!(date("thursday 29 Feb 2024", "%A %d %b %Y"), Some(19_782.0));
        assert_eq!(date("Fri 29 Feb 2024", "%a %d %b %Y"), None);
        assert_eq!(date("01/01/69", "%d/%m/%y"), Some(-365.0));
        assert_eq!(
            date("01/01/68", "%d/%m/%y"),
            Some(days_from_civil(2068, 1, 1) as f64)
        );
        assert_eq!(
            date("2000-01-01 12:00:00", "%Y-%m-%d %H:%M:%S"),
            Some(10_957.5)
        );
        assert_eq!(date("1969-12-31 18:00", "%Y-%m-%d %H:%M"), Some(-0.25));
        assert_eq!(date("12:00", "%H:%M"), Some(0.5));
        assert_eq!(
            date("100% 2024", "100%% %Y"),
            Some(days_from_civil(2024, 1, 1) as f64)
        );
        assert_eq!(
            date("%q 2024", "%q %Y"),
            Some(days_from_civil(2024, 1, 1) as f64)
        );
        for (bad, pat) in [
            ("2023-02-29", p),
            ("2024-02-30", p),
            ("1900-02-29", p),
            ("2024-13-01", p),
            ("2024-00-01", p),
            ("2024-01-00", p),
            ("2024-04-31", p),
            ("2024-01-01x", p),
            ("2024/01/01", p),
            ("", p),
            ("2024-01", p),
            ("2024-01-01 24:00:00", "%Y-%m-%d %H:%M:%S"),
            ("2024-01-01 23:60:00", "%Y-%m-%d %H:%M:%S"),
            ("2024-01-01 23:59:60", "%Y-%m-%d %H:%M:%S"),
            ("1 Foo 2024", "%d %b %Y"),
            ("001", "%j"),
            ("5/1/2024", "%d/%m/%y"),
            ("abc", p),
        ] {
            assert_eq!(date(bad, pat), None, "{bad:?} / {pat:?}");
        }
        // 2000 was a leap year; 1900 and 2100 were not.
        assert!(date("2000-02-29", p).is_some());
        assert!(date("2100-02-29", p).is_none());
        assert!(date("1600-02-29", p).is_some());
    }

    #[test]
    fn currency_golden() {
        assert_eq!(currency("$1,234.50"), Some(1234.5));
        assert_eq!(currency("(12.50)"), Some(-12.5));
        assert_eq!(currency("($12.50)"), Some(-12.5));
        assert_eq!(currency("-$3"), Some(-3.0));
        assert_eq!(currency("$-3"), Some(-3.0));
        assert_eq!(currency("\u{2212}$3"), Some(-3.0));
        assert_eq!(currency("+$3"), Some(3.0));
        assert_eq!(currency("€ 12"), Some(12.0));
        assert_eq!(currency("12.50 €"), Some(12.5));
        assert_eq!(currency("£0.99"), Some(0.99));
        assert_eq!(currency("¥1000"), Some(1000.0));
        assert_eq!(currency("AUD 1,000"), Some(1000.0));
        assert_eq!(currency("1,000 USD"), Some(1000.0));
        assert_eq!(currency("USD1000"), Some(1000.0));
        assert_eq!(currency("US$5"), Some(5.0));
        assert_eq!(currency("A$5"), Some(5.0));
        assert_eq!(currency("-A$5"), Some(-5.0));
        assert_eq!(currency("1 234.5"), Some(1234.5));
        assert_eq!(currency("1\u{a0}234"), Some(1234.0));
        assert_eq!(currency("  42 "), Some(42.0));
        assert_eq!(currency(".5"), Some(0.5));
        assert_eq!(currency("0"), Some(0.0));
        for bad in [
            "",
            "$",
            "()",
            "(-12)",
            "-(12)",
            "--3",
            "$$3",
            "AUD $12",
            "$12 USD",
            "12-",
            "abc",
            "usd 12",
            "1.234,50 €",
            "$inf",
            "$nan",
            "$1e999",
            "(12",
            "12)",
            "ABCD 12",
            "$ - 3 -",
        ] {
            assert_eq!(currency(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn dms_round_trip_golden() {
        for v in [
            0.0, -35.3075, 149.1286, -90.0, 180.0, 0.5, -0.000_1, 89.999_999,
        ] {
            for kind in [DmsKind::Lat, DmsKind::Lon, DmsKind::Signed] {
                for dp in [0, 1, 2, 4] {
                    let tol = 0.5 * 10f64.powi(-(dp as i32)) / 3600.0 + 1e-9;
                    let s = fmt::dms(v, kind, dp);
                    let back = dms(&s);
                    assert!(
                        back.is_some_and(|b| (b - v).abs() <= tol),
                        "{v} {kind:?} {dp}: {s:?} -> {back:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn date_round_trip_leap_and_pre_1970() {
        for (y, m, d) in [
            (1600, 2, 29),
            (1900, 2, 28),
            (1900, 3, 1),
            (1969, 12, 31),
            (1970, 1, 1),
            (2000, 2, 29),
            (2024, 2, 29),
            (2100, 3, 1),
            (0, 2, 29),
            (-4, 2, 29),
            (-1, 12, 31),
        ] {
            let z = days_from_civil(y, m, d) as f64;
            for p in ["%Y-%m-%d", "%d %b %Y", "%Y%m%d", "%a %d %B %Y"] {
                assert_eq!(date(&fmt::date(z, p), p), Some(z), "{y}-{m}-{d} {p}");
            }
        }
    }

    proptest! {
        #[test]
        fn prop_dms_round_trip(v in -400.0f64..400.0, kind in 0usize..3, dp in 0u32..=6) {
            let kind = [DmsKind::Lat, DmsKind::Lon, DmsKind::Signed][kind];
            let s = fmt::dms(v, kind, dp);
            let back = dms(&s);
            let tol = 0.5 * 10f64.powi(-(dp as i32)) / 3600.0 + 1e-9;
            prop_assert!(back.is_some_and(|b| (b - v).abs() <= tol), "{} -> {:?} -> {:?}", v, s, back);
        }

        #[test]
        fn prop_date_round_trip(z in -1_000_000i64..1_000_000, p in 0usize..6) {
            let p = ["%Y-%m-%d", "%d/%m/%Y", "%d %b %Y", "%B %d, %Y", "%Y%m%d", "%A %Y-%m-%d"][p];
            let z = z as f64;
            let s = fmt::date(z, p);
            prop_assert_eq!(date(&s, p), Some(z), "{}", s);
        }

        #[test]
        fn prop_date_time_round_trip(z in -100_000i64..100_000, secs in 0i64..86_400) {
            let p = "%Y-%m-%d %H:%M:%S";
            let v = z as f64 + secs as f64 / 86_400.0;
            let back = date(&fmt::date(v, p), p);
            prop_assert!(back.is_some_and(|b| (b - v).abs() < 1e-9));
        }

        #[test]
        fn prop_number_round_trip(v in proptest::num::f64::NORMAL | proptest::num::f64::ZERO | proptest::num::f64::SUBNORMAL) {
            prop_assert_eq!(number(&v.to_string()), Some(v));
            prop_assert_eq!(number(&format!("{v:e}")), Some(v));
        }

        #[test]
        fn prop_thousands_parse(v in -1e12f64..1e12, dp in 0u32..4) {
            let s = fmt::thousands(v, dp);
            let back = number(&s);
            let tol = 0.5 * 10f64.powi(-(dp as i32)) + 1e-3;
            prop_assert!(back.is_some_and(|b| (b - v).abs() <= tol), "{} -> {}", v, s);
        }

        #[test]
        fn prop_currency_round_trip(v in -1e9f64..1e9) {
            let s = fmt::currency(v, "$", 2);
            let back = currency(&s);
            prop_assert!(back.is_some_and(|b| (b - v).abs() <= 0.005 + 1e-6), "{} -> {}", v, s);
        }

        #[test]
        fn prop_parsers_never_panic(s in "\\PC{0,24}") {
            let _ = number(&s);
            let _ = dms(&s);
            let _ = date(&s, "%Y-%m-%d %H:%M:%S %b %a %%");
            let _ = currency(&s);
        }
    }
}
