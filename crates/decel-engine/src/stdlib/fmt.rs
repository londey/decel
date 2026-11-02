// SPDX-License-Identifier: MIT OR Apache-2.0

//! Display formatting exposed to Lua as `fmt.*` (DESIGN §4.5).
//!
//! Every function returns a `String`. Non-finite input always formats as `"NaN"`, `"∞"` or
//! `"-∞"`, whatever the function.

use std::fmt::Write as _;

/// Largest number of decimal places honoured by [`fixed`], [`thousands`], [`percent`] and
/// [`currency`]. Larger requests are clamped.
pub const MAX_DP: u32 = 20;

/// Largest number of decimal places on the seconds field honoured by [`dms`].
pub const MAX_SEC_DP: u32 = 9;

/// Largest `|days|` accepted by [`date`] (about 2.7 billion years either side of 1970).
pub const MAX_DATE_DAYS: f64 = 1e12;

/// Output of [`date`] when `|days|` exceeds [`MAX_DATE_DAYS`].
pub const DATE_OUT_OF_RANGE: &str = "####";

pub(crate) const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
pub(crate) const MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
pub(crate) const DAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub(crate) const DAYS_LONG: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Engineering prefixes from 10^-24 to 10^24, indexed by `exponent / 3 + 8`.
const SI_PREFIXES: [&str; 17] = [
    "y", "z", "a", "f", "p", "n", "µ", "m", "", "k", "M", "G", "T", "P", "E", "Z", "Y",
];

fn non_finite(v: f64) -> Option<String> {
    if v.is_nan() {
        Some("NaN".to_owned())
    } else if v.is_infinite() {
        Some(if v > 0.0 { "∞" } else { "-∞" }.to_owned())
    } else {
        None
    }
}

/// Splits a leading `-` off a formatted number.
fn split_sign(s: &str) -> (&str, &str) {
    match s.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", s),
    }
}

/// Fixed-point with `dp` decimals: `fixed(1234.567, 2) == "1234.57"`.
///
/// Rounding is exact on the binary value (so `1.005` is `1.00`); exact ties round half to even.
/// A result that rounds to zero never carries a sign.
pub fn fixed(v: f64, dp: u32) -> String {
    if let Some(s) = non_finite(v) {
        return s;
    }
    let dp = dp.min(MAX_DP) as usize;
    let s = format!("{v:.dp$}");
    match s.strip_prefix('-') {
        Some(rest) if rest.bytes().all(|b| b == b'0' || b == b'.') => rest.to_owned(),
        _ => s,
    }
}

fn group_thousands(int_digits: &str) -> String {
    let n = int_digits.len();
    let mut out = String::with_capacity(n + n / 3);
    for (i, ch) in int_digits.chars().enumerate() {
        if i > 0 && (n - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Like [`fixed`] with `,` thousands separators: `thousands(1234567.891, 2) == "1,234,567.89"`.
pub fn thousands(v: f64, dp: u32) -> String {
    if let Some(s) = non_finite(v) {
        return s;
    }
    let s = fixed(v, dp);
    let (sign, body) = split_sign(&s);
    let (int, frac) = match body.find('.') {
        Some(i) => body.split_at(i),
        None => (body, ""),
    };
    format!("{sign}{}{frac}", group_thousands(int))
}

/// `v * 100` with `dp` decimals and a `%` suffix: `percent(0.125, 1) == "12.5%"`.
pub fn percent(v: f64, dp: u32) -> String {
    let scaled = v * 100.0;
    if let Some(s) = non_finite(scaled) {
        return s;
    }
    format!("{}%", fixed(scaled, dp))
}

/// Scientific notation with `sig` significant figures (clamped to 1..=17):
/// `sci(12345.0, 3) == "1.23e4"`. Trailing zeros are kept (`sci(1e4, 3) == "1.00e4"`);
/// zero is `"0"`.
pub fn sci(v: f64, sig: u32) -> String {
    if let Some(s) = non_finite(v) {
        return s;
    }
    if v == 0.0 {
        return "0".to_owned();
    }
    let p = (sig.clamp(1, 17) - 1) as usize;
    format!("{v:.p$e}")
}

fn with_unit(num: &str, prefix: &str, unit: &str) -> String {
    if unit.is_empty() {
        format!("{num}{prefix}")
    } else {
        format!("{num} {prefix}{unit}")
    }
}

/// Engineering-prefixed display with `sig` significant figures (clamped to 1..=17):
/// `si(12500.0, "m", 3) == "12.5 km"`. Trailing zeros are trimmed. Magnitudes outside
/// the `y`..`Y` prefixes fall back to [`sci`] followed by the unit.
pub fn si(v: f64, unit: &str, sig: u32) -> String {
    if let Some(s) = non_finite(v) {
        return s;
    }
    let sig = sig.clamp(1, 17);
    if v == 0.0 {
        return with_unit("0", "", unit);
    }
    let fallback = || with_unit(&sci(v, sig), "", unit);
    let p = (sig - 1) as usize;
    // Round first, so the exponent reflects any carry (999.96 -> 1.00e3).
    let rounded = format!("{:.p$e}", v.abs());
    let Some((mantissa, exp)) = rounded.split_once('e') else {
        return fallback();
    };
    let Ok(exp) = exp.parse::<i32>() else {
        return fallback();
    };
    let eng = exp.div_euclid(3) * 3;
    let Some(prefix) = usize::try_from(eng / 3 + 8)
        .ok()
        .filter(|_| (-24..=24).contains(&eng))
        .and_then(|i| SI_PREFIXES.get(i))
    else {
        return fallback();
    };
    let mut digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let int_len = (exp - eng + 1) as usize; // 1..=3
    while digits.len() < int_len {
        digits.push('0');
    }
    let (int_part, frac_part) = digits.split_at(int_len);
    let frac_part = frac_part.trim_end_matches('0');
    let sign = if v < 0.0 { "-" } else { "" };
    let num = if frac_part.is_empty() {
        format!("{sign}{int_part}")
    } else {
        format!("{sign}{int_part}.{frac_part}")
    };
    with_unit(&num, prefix, unit)
}

/// [`thousands`] with a currency symbol after the sign: `currency(-1234.5, "$", 2) == "-$1,234.50"`.
pub fn currency(v: f64, symbol: &str, dp: u32) -> String {
    if let Some(s) = non_finite(v) {
        return s;
    }
    let s = thousands(v, dp);
    let (sign, body) = split_sign(&s);
    format!("{sign}{symbol}{body}")
}

/// Hemisphere style for [`dms`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmsKind {
    /// Latitude: `N` for values >= 0, `S` for negative values.
    Lat,
    /// Longitude: `E` for values >= 0, `W` for negative values.
    Lon,
    /// No hemisphere letter; negative values get a leading `-`.
    Signed,
}

/// Degrees, minutes and seconds: `dms(-35.3075, DmsKind::Lat, 0) == "35°18′27″S"`.
///
/// Seconds carry exactly `sec_dp` decimals (clamped to [`MAX_SEC_DP`]); rounding carries
/// into minutes and degrees. Minutes and seconds are not zero-padded. A value that rounds
/// to zero is treated as non-negative.
pub fn dms(v: f64, kind: DmsKind, sec_dp: u32) -> String {
    if let Some(s) = non_finite(v) {
        return s;
    }
    let dp = sec_dp.min(MAX_SEC_DP);
    let scale = 10u128.pow(dp);
    let units_f = (v.abs() * 3600.0 * scale as f64).round();
    let (body, is_zero) = if units_f < 1e36 {
        let units = units_f as u128;
        let per_min = 60 * scale;
        let per_deg = 3600 * scale;
        let deg = units / per_deg;
        let min = units % per_deg / per_min;
        let sec_units = units % per_min;
        let sec_int = sec_units / scale;
        let sec = if dp == 0 {
            sec_int.to_string()
        } else {
            let frac = sec_units % scale;
            let width = dp as usize;
            format!("{sec_int}.{frac:0width$}")
        };
        (format!("{deg}°{min}′{sec}″"), units == 0)
    } else {
        // Beyond sub-degree precision of the integer path: whole degrees only.
        let zero_sec = if dp == 0 {
            "0".to_owned()
        } else {
            format!("0.{}", "0".repeat(dp as usize))
        };
        (format!("{}°0′{zero_sec}″", fixed(v.abs(), 0)), false)
    };
    let negative = v < 0.0 && !is_zero;
    match kind {
        DmsKind::Lat => format!("{body}{}", if negative { 'S' } else { 'N' }),
        DmsKind::Lon => format!("{body}{}", if negative { 'W' } else { 'E' }),
        DmsKind::Signed => format!("{}{body}", if negative { "-" } else { "" }),
    }
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to (year, month, day),
/// proleptic Gregorian.
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

/// Howard Hinnant's `days_from_civil`: inverse of [`civil_from_days`].
pub(crate) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let d = i64::from(d);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(crate) fn is_leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

pub(crate) fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Weekday index for a day number, 0 = Sunday.
pub(crate) fn weekday(z: i64) -> usize {
    (z + 4).rem_euclid(7) as usize
}

fn name(table: &[&'static str], i: usize) -> &'static str {
    table.get(i).copied().unwrap_or("?")
}

/// Formats `days` since 1970-01-01 UTC (fractional part = time of day) with a strftime subset:
/// `%Y %m %d %H %M %S %y %b %B %a %A %j %%`. Unknown sequences are emitted literally.
///
/// The time of day is rounded to the nearest second. Years before 1 are astronomical
/// (`0000` = 1 BC, `-0001` = 2 BC). `|days|` beyond [`MAX_DATE_DAYS`] gives
/// [`DATE_OUT_OF_RANGE`].
pub fn date(days: f64, pattern: &str) -> String {
    if let Some(s) = non_finite(days) {
        return s;
    }
    if days.abs() > MAX_DATE_DAYS {
        return DATE_OUT_OF_RANGE.to_owned();
    }
    let mut day = days.floor();
    let mut secs = ((days - day) * 86_400.0).round();
    if secs >= 86_400.0 {
        day += 1.0;
        secs -= 86_400.0;
    }
    let z = day as i64;
    let secs = secs as u32;
    let (y, m, d) = civil_from_days(z);
    let (hh, mm, ss) = (secs / 3600, secs / 60 % 60, secs % 60);
    let month_index = (m as usize).saturating_sub(1);
    let wd = weekday(z);
    let yday = z - days_from_civil(y, 1, 1) + 1;

    let mut out = String::with_capacity(pattern.len() + 8);
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        // Writing to a String cannot fail.
        let _ = match chars.next() {
            Some('Y') if y < 0 => write!(out, "-{:04}", y.unsigned_abs()),
            Some('Y') => write!(out, "{y:04}"),
            Some('y') => write!(out, "{:02}", y.rem_euclid(100)),
            Some('m') => write!(out, "{m:02}"),
            Some('d') => write!(out, "{d:02}"),
            Some('H') => write!(out, "{hh:02}"),
            Some('M') => write!(out, "{mm:02}"),
            Some('S') => write!(out, "{ss:02}"),
            Some('j') => write!(out, "{yday:03}"),
            Some('b') => write!(out, "{}", name(&MONTHS_SHORT, month_index)),
            Some('B') => write!(out, "{}", name(&MONTHS_LONG, month_index)),
            Some('a') => write!(out, "{}", name(&DAYS_SHORT, wd)),
            Some('A') => write!(out, "{}", name(&DAYS_LONG, wd)),
            Some('%') => write!(out, "%"),
            Some(other) => write!(out, "%{other}"),
            None => write!(out, "%"),
        };
    }
    out
}

/// `[-][Nd ]H:MM:SS` from seconds: `duration(90061.0) == "1d 1:01:01"`.
///
/// Rounds to the nearest second (half away from zero); a result of zero has no sign.
/// Magnitudes beyond `u64::MAX` seconds saturate.
pub fn duration(seconds: f64) -> String {
    if let Some(s) = non_finite(seconds) {
        return s;
    }
    let r = seconds.round();
    let total = r.abs() as u64;
    let sign = if r < 0.0 && total > 0 { "-" } else { "" };
    let (d, h, m, s) = (
        total / 86_400,
        total / 3600 % 24,
        total / 60 % 60,
        total % 60,
    );
    if d > 0 {
        format!("{sign}{d}d {h}:{m:02}:{s:02}")
    } else {
        format!("{sign}{h}:{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INF: f64 = f64::INFINITY;

    fn assert_non_finite(f: impl Fn(f64) -> String) {
        assert_eq!(f(f64::NAN), "NaN");
        assert_eq!(f(INF), "∞");
        assert_eq!(f(-INF), "-∞");
    }

    #[test]
    fn non_finite_everywhere() {
        assert_non_finite(|v| fixed(v, 2));
        assert_non_finite(|v| thousands(v, 2));
        assert_non_finite(|v| percent(v, 1));
        assert_non_finite(|v| sci(v, 3));
        assert_non_finite(|v| si(v, "m", 3));
        assert_non_finite(|v| currency(v, "$", 2));
        assert_non_finite(|v| dms(v, DmsKind::Lat, 0));
        assert_non_finite(|v| dms(v, DmsKind::Signed, 2));
        assert_non_finite(|v| date(v, "%Y-%m-%d"));
        assert_non_finite(duration);
    }

    #[test]
    fn fixed_golden() {
        assert_eq!(fixed(1234.567, 2), "1234.57");
        assert_eq!(fixed(-1234.567, 2), "-1234.57");
        assert_eq!(fixed(0.0, 2), "0.00");
        assert_eq!(fixed(-0.0, 2), "0.00");
        assert_eq!(fixed(-0.001, 2), "0.00");
        assert_eq!(fixed(-0.4, 0), "0");
        assert_eq!(fixed(2.5, 0), "2");
        assert_eq!(fixed(3.5, 0), "4");
        assert_eq!(fixed(1.005, 2), "1.00"); // binary value is 1.00499...
        assert_eq!(fixed(9.999, 2), "10.00");
        assert_eq!(fixed(1e21, 0), "1000000000000000000000");
        assert_eq!(fixed(1e-10, 3), "0.000");
        assert_eq!(fixed(1.0, 1000).len(), 2 + MAX_DP as usize);
    }

    #[test]
    fn thousands_golden() {
        assert_eq!(thousands(1_234_567.891, 2), "1,234,567.89");
        assert_eq!(thousands(-1000.0, 0), "-1,000");
        assert_eq!(thousands(999.0, 0), "999");
        assert_eq!(thousands(999.5, 0), "1,000");
        assert_eq!(thousands(100_000.0, 1), "100,000.0");
        assert_eq!(thousands(-0.0, 2), "0.00");
        assert_eq!(thousands(-0.001, 2), "0.00");
        assert_eq!(thousands(0.5, 2), "0.50");
        assert_eq!(thousands(1e15, 0), "1,000,000,000,000,000");
    }

    #[test]
    fn percent_golden() {
        assert_eq!(percent(0.125, 1), "12.5%");
        assert_eq!(percent(-0.5, 0), "-50%");
        assert_eq!(percent(0.0, 2), "0.00%");
        assert_eq!(percent(-0.0, 0), "0%");
        assert_eq!(percent(1.0, 0), "100%");
        assert_eq!(percent(0.99999, 2), "100.00%");
        assert_eq!(percent(1e307, 0), "∞");
    }

    #[test]
    fn sci_golden() {
        assert_eq!(sci(12345.0, 3), "1.23e4");
        assert_eq!(sci(-12345.0, 3), "-1.23e4");
        assert_eq!(sci(0.0, 3), "0");
        assert_eq!(sci(-0.0, 3), "0");
        assert_eq!(sci(1e4, 3), "1.00e4");
        assert_eq!(sci(9.999, 3), "1.00e1");
        assert_eq!(sci(0.000_012_345, 2), "1.2e-5");
        assert_eq!(sci(1.5, 0), "2e0"); // sig clamped to 1
        assert_eq!(sci(1.0 / 3.0, 100), "3.3333333333333331e-1"); // clamped to 17
        assert_eq!(sci(1e300, 1), "1e300");
        assert_eq!(sci(5e-324, 2), "4.9e-324");
    }

    #[test]
    fn si_golden() {
        assert_eq!(si(12500.0, "m", 3), "12.5 km");
        assert_eq!(si(-12500.0, "m", 3), "-12.5 km");
        assert_eq!(si(12500.0, "", 3), "12.5k");
        assert_eq!(si(5.0, "", 3), "5");
        assert_eq!(si(5.0, "m", 3), "5 m");
        assert_eq!(si(0.0, "m", 3), "0 m");
        assert_eq!(si(-0.0, "m", 3), "0 m");
        assert_eq!(si(0.0, "", 3), "0");
        assert_eq!(si(999.96, "V", 3), "1 kV");
        assert_eq!(si(999.94, "V", 4), "999.9 V");
        assert_eq!(si(0.000_001_5, "F", 3), "1.5 µF");
        assert_eq!(si(0.0047, "F", 2), "4.7 mF");
        assert_eq!(si(1.0, "g", 3), "1 g");
        assert_eq!(si(12500.0, "m", 1), "10 km");
        assert_eq!(si(123_456.0, "Hz", 2), "120 kHz");
        assert_eq!(si(1e24, "m", 3), "1 Ym");
        assert_eq!(si(1e-24, "m", 3), "1 ym");
        assert_eq!(si(999.9e24, "m", 3), "1.00e27 m");
        assert_eq!(si(1e27, "m", 3), "1.00e27 m");
        assert_eq!(si(1e-25, "m", 2), "1.0e-25 m");
        assert_eq!(si(1e-25, "", 2), "1.0e-25");
        assert_eq!(si(1.23456789e9, "B", 17), "1.23456789 GB");
    }

    #[test]
    fn currency_golden() {
        assert_eq!(currency(-1234.5, "$", 2), "-$1,234.50");
        assert_eq!(currency(1234.5, "$", 2), "$1,234.50");
        assert_eq!(currency(0.0, "€", 2), "€0.00");
        assert_eq!(currency(-0.0, "€", 2), "€0.00");
        assert_eq!(currency(-0.001, "$", 2), "$0.00");
        assert_eq!(currency(1e6, "AUD ", 0), "AUD 1,000,000");
        assert_eq!(currency(12.0, "", 2), "12.00");
    }

    fn lat_of(d: f64, m: f64, s: f64) -> f64 {
        d + m / 60.0 + s / 3600.0
    }

    #[test]
    fn dms_golden() {
        assert_eq!(dms(-35.3075, DmsKind::Lat, 0), "35°18′27″S");
        assert_eq!(dms(35.3075, DmsKind::Lat, 0), "35°18′27″N");
        assert_eq!(dms(149.1286, DmsKind::Lon, 1), "149°7′43.0″E");
        assert_eq!(dms(149.1286, DmsKind::Lon, 2), "149°7′42.96″E");
        assert_eq!(dms(-149.1286, DmsKind::Lon, 2), "149°7′42.96″W");
        assert_eq!(dms(-35.3075, DmsKind::Signed, 0), "-35°18′27″");
        assert_eq!(dms(35.3075, DmsKind::Signed, 0), "35°18′27″");
        assert_eq!(dms(0.0, DmsKind::Lat, 0), "0°0′0″N");
        assert_eq!(dms(-0.0, DmsKind::Lat, 0), "0°0′0″N");
        assert_eq!(dms(-0.0, DmsKind::Lon, 1), "0°0′0.0″E");
        assert_eq!(dms(-0.0, DmsKind::Signed, 0), "0°0′0″");
        // Rounds to zero -> non-negative.
        assert_eq!(dms(-1e-9, DmsKind::Lat, 0), "0°0′0″N");
        assert_eq!(dms(-1e-9, DmsKind::Signed, 2), "0°0′0.00″");
        assert_eq!(dms(lat_of(1.0, 2.0, 3.0), DmsKind::Signed, 0), "1°2′3″");
        assert_eq!(dms(lat_of(0.0, 0.0, 0.5), DmsKind::Signed, 3), "0°0′0.500″");
        assert_eq!(dms(-90.0, DmsKind::Lat, 0), "90°0′0″S");
        assert_eq!(dms(180.0, DmsKind::Lon, 0), "180°0′0″E");
        assert_eq!(dms(400.0, DmsKind::Signed, 0), "400°0′0″");
        assert_eq!(dms(1.0, DmsKind::Signed, 50), "1°0′0.000000000″");
        assert_eq!(
            dms(1e40, DmsKind::Signed, 0),
            format!("{}°0′0″", fixed(1e40, 0))
        );
        assert_eq!(
            dms(-1e40, DmsKind::Lat, 1),
            format!("{}°0′0.0″S", fixed(1e40, 0))
        );
    }

    #[test]
    fn dms_rounding_carries() {
        // Seconds that round up to 60 carry into minutes, and minutes into degrees.
        let v = lat_of(10.0, 59.0, 59.99995);
        assert_eq!(dms(v, DmsKind::Lat, 0), "11°0′0″N");
        assert_eq!(dms(v, DmsKind::Lat, 1), "11°0′0.0″N");
        assert_eq!(dms(-v, DmsKind::Lat, 1), "11°0′0.0″S");
        let v4 = lat_of(10.0, 59.0, 59.99996);
        assert_eq!(dms(v4, DmsKind::Lat, 4), "11°0′0.0000″N");
        assert_eq!(dms(-v4, DmsKind::Signed, 4), "-11°0′0.0000″");
        // Just below the threshold at 4dp.
        let v4b = lat_of(10.0, 59.0, 59.99994);
        assert_eq!(dms(v4b, DmsKind::Lat, 4), "10°59′59.9999″N");
        // Carry into minutes only.
        let m = lat_of(10.0, 5.0, 59.6);
        assert_eq!(dms(m, DmsKind::Lon, 0), "10°6′0″E");
        assert_eq!(dms(lat_of(10.0, 5.0, 59.96), DmsKind::Lon, 1), "10°6′0.0″E");
        assert_eq!(
            dms(lat_of(10.0, 5.0, 59.94), DmsKind::Lon, 1),
            "10°5′59.9″E"
        );
        // Carry from just below a whole degree up to the next.
        assert_eq!(dms(-0.999_999_99, DmsKind::Lat, 0), "1°0′0″S");
    }

    #[test]
    fn hinnant_known_days() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(-25_567), (1900, 1, 1));
        assert_eq!(civil_from_days(-719_528), (0, 1, 1));
        assert_eq!(
            days_from_civil(1900, 3, 1) - days_from_civil(1900, 2, 28),
            1
        );
        assert_eq!(
            days_from_civil(2000, 3, 1) - days_from_civil(2000, 2, 28),
            2
        );
        for z in -800_000..800_000 {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
            assert!(d >= 1 && d <= days_in_month(y, m));
        }
    }

    #[test]
    fn date_golden() {
        let p = "%Y-%m-%d";
        assert_eq!(date(0.0, p), "1970-01-01");
        assert_eq!(date(-0.0, p), "1970-01-01");
        assert_eq!(date(-1.0, p), "1969-12-31");
        assert_eq!(date(-0.25, "%Y-%m-%d %H:%M:%S"), "1969-12-31 18:00:00");
        assert_eq!(date(19_782.0, p), "2024-02-29");
        assert_eq!(date(10_957.5, "%Y-%m-%d %H:%M:%S"), "2000-01-01 12:00:00");
        assert_eq!(date(-25_567.0, p), "1900-01-01");
        assert_eq!(date(-719_528.0, p), "0000-01-01");
        assert_eq!(date(-719_529.0, p), "-0001-12-31");
        assert_eq!(date(2_932_896.0, p), "9999-12-31");
        assert_eq!(date(2_932_897.0, p), "10000-01-01");
        assert_eq!(
            date(0.0, "%a %A %b %B %j %y"),
            "Thu Thursday Jan January 001 70"
        );
        assert_eq!(
            date(19_782.0, "%a %d %b %Y, day %j"),
            "Thu 29 Feb 2024, day 060"
        );
        assert_eq!(date(-1.0, "%A %j %y"), "Wednesday 365 69");
        assert_eq!(date(days_from_civil(2024, 12, 31) as f64, "%j"), "366");
        assert_eq!(date(days_from_civil(-1, 6, 1) as f64, "%y"), "99");
        assert_eq!(date(0.0, "100%% %q %"), "100% %q %");
        assert_eq!(date(0.0, "über %d"), "über 01");
        // Time of day rounds to the nearest second and can carry into the next day.
        assert_eq!(
            date(0.999_999_99, "%Y-%m-%d %H:%M:%S"),
            "1970-01-02 00:00:00"
        );
        assert_eq!(date(1.0 / 86_400.0 * 3723.4, "%H:%M:%S"), "01:02:03");
        assert_eq!(date(1e13, p), DATE_OUT_OF_RANGE);
        assert_eq!(date(-1e13, p), DATE_OUT_OF_RANGE);
        assert_ne!(date(1e12, p), DATE_OUT_OF_RANGE);
        assert_ne!(date(-1e12, p), DATE_OUT_OF_RANGE);
    }

    #[test]
    fn duration_golden() {
        assert_eq!(duration(3723.0), "1:02:03");
        assert_eq!(duration(90_061.0), "1d 1:01:01");
        assert_eq!(duration(-5.0), "-0:00:05");
        assert_eq!(duration(0.0), "0:00:00");
        assert_eq!(duration(-0.0), "0:00:00");
        assert_eq!(duration(-0.4), "0:00:00");
        assert_eq!(duration(0.5), "0:00:01");
        assert_eq!(duration(59.6), "0:01:00");
        assert_eq!(duration(86_399.6), "1d 0:00:00");
        assert_eq!(duration(-90_061.0), "-1d 1:01:01");
        assert_eq!(duration(86_400.0 * 400.0 + 36_000.0), "400d 10:00:00");
        assert_eq!(duration(1e30), duration(u64::MAX as f64));
    }

    #[test]
    fn group_thousands_lengths() {
        assert_eq!(group_thousands("1"), "1");
        assert_eq!(group_thousands("12"), "12");
        assert_eq!(group_thousands("123"), "123");
        assert_eq!(group_thousands("1234"), "1,234");
        assert_eq!(group_thousands("123456"), "123,456");
        assert_eq!(group_thousands("1234567"), "1,234,567");
    }
}
