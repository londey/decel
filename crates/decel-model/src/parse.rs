// SPDX-License-Identifier: MIT OR Apache-2.0

//! The default value parser (DESIGN §3.3).

use crate::Value;

/// Parses typed text into a raw value when a cell has no `parse` hook.
///
/// In order: a leading `'` forces a string; blank text clears the value;
/// `true`/`false` (any case); an integer; a float (`1e3`, `-0.5`, `1_000`,
/// `1,000.5`); otherwise the text as a string. Integers that overflow `i64`
/// become floats. `inf` and `nan` are strings.
pub fn parse_default(text: &str) -> Option<Value> {
    if let Some(rest) = text.strip_prefix('\'') {
        return Some(Value::str(rest));
    }
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if t.eq_ignore_ascii_case("true") {
        return Some(Value::Bool(true));
    }
    if t.eq_ignore_ascii_case("false") {
        return Some(Value::Bool(false));
    }
    Some(parse_number(t).unwrap_or_else(|| Value::str(text)))
}

fn parse_number(t: &str) -> Option<Value> {
    let (sign, body) = match t.as_bytes().first()? {
        b'-' => ("-", &t[1..]),
        b'+' => ("", &t[1..]),
        _ => ("", t),
    };
    if !underscores_ok(body) {
        return None;
    }
    let body = body.replace('_', "");
    let int_end = body
        .find(|ch: char| !(ch.is_ascii_digit() || ch == ','))
        .unwrap_or(body.len());
    let (int_part, rest) = body.split_at(int_end);
    let int_digits = strip_thousands(int_part)?;

    if rest.is_empty() {
        if int_digits.is_empty() {
            return None;
        }
        let s = format!("{sign}{int_digits}");
        return Some(match s.parse::<i64>() {
            Ok(i) => Value::Int(i),
            Err(_) => Value::Num(s.parse::<f64>().ok()?),
        });
    }

    let (frac, exp) = split_fraction_exponent(rest)?;
    if int_digits.is_empty() && frac.is_empty() {
        return None;
    }
    let s = format!("{sign}{int_digits}.{frac}{exp}");
    s.parse::<f64>().ok().map(Value::Num)
}

/// Underscores are allowed only between two digits.
fn underscores_ok(s: &str) -> bool {
    let b = s.as_bytes();
    b.iter().enumerate().all(|(i, &ch)| {
        ch != b'_'
            || (i > 0 && i + 1 < b.len() && b[i - 1].is_ascii_digit() && b[i + 1].is_ascii_digit())
    })
}

/// Validates `1,234,567`-style grouping and returns the bare digits.
fn strip_thousands(int_part: &str) -> Option<String> {
    if !int_part.contains(',') {
        return Some(int_part.to_string());
    }
    let mut groups = int_part.split(',');
    let first = groups.next()?;
    if first.is_empty() || first.len() > 3 {
        return None;
    }
    let mut out = first.to_string();
    for g in groups {
        if g.len() != 3 {
            return None;
        }
        out.push_str(g);
    }
    Some(out)
}

/// Splits `.123e-4` into (`123`, `e-4`). Returns `None` for anything else.
fn split_fraction_exponent(rest: &str) -> Option<(&str, &str)> {
    let (frac, exp) = match rest.find(['e', 'E']) {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let frac = match frac.strip_prefix('.') {
        Some(f) => f,
        None if frac.is_empty() => "",
        None => return None,
    };
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !exp.is_empty() {
        let digits = exp[1..].strip_prefix(['+', '-']).unwrap_or(&exp[1..]);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    Some((frac, exp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn p(s: &str) -> Option<Value> {
        parse_default(s)
    }

    #[test]
    fn blanks_clear() {
        assert_eq!(p(""), None);
        assert_eq!(p("   "), None);
        assert_eq!(p("\t\n"), None);
    }

    #[test]
    fn quote_forces_string() {
        assert_eq!(p("'42"), Some(Value::str("42")));
        assert_eq!(p("'"), Some(Value::str("")));
        assert_eq!(p("''x"), Some(Value::str("'x")));
        assert_eq!(p("' true"), Some(Value::str(" true")));
    }

    #[test]
    fn booleans() {
        assert_eq!(p("true"), Some(Value::Bool(true)));
        assert_eq!(p("FALSE"), Some(Value::Bool(false)));
        assert_eq!(p(" True "), Some(Value::Bool(true)));
        assert_eq!(p("yes"), Some(Value::str("yes")));
    }

    #[test]
    fn integers() {
        assert_eq!(p("0"), Some(Value::Int(0)));
        assert_eq!(p("-17"), Some(Value::Int(-17)));
        assert_eq!(p("+5"), Some(Value::Int(5)));
        assert_eq!(p(" 42 "), Some(Value::Int(42)));
        assert_eq!(p("1_000"), Some(Value::Int(1000)));
        assert_eq!(p("1,000"), Some(Value::Int(1000)));
        assert_eq!(p("12,345,678"), Some(Value::Int(12_345_678)));
        assert_eq!(p("007"), Some(Value::Int(7)));
        assert_eq!(p("9223372036854775807"), Some(Value::Int(i64::MAX)));
        assert_eq!(p("-9223372036854775808"), Some(Value::Int(i64::MIN)));
        assert_eq!(
            p("9223372036854775808"),
            Some(Value::Num(9.223372036854776e18))
        );
    }

    #[test]
    fn floats() {
        assert_eq!(p("1.5"), Some(Value::Num(1.5)));
        assert_eq!(p("-0.5"), Some(Value::Num(-0.5)));
        assert_eq!(p(".5"), Some(Value::Num(0.5)));
        assert_eq!(p("5."), Some(Value::Num(5.0)));
        assert_eq!(p("1e3"), Some(Value::Num(1000.0)));
        assert_eq!(p("1E-3"), Some(Value::Num(0.001)));
        assert_eq!(p("2.5e+2"), Some(Value::Num(250.0)));
        assert_eq!(p("1,000.25"), Some(Value::Num(1000.25)));
        assert_eq!(p("1_000.000_1"), Some(Value::Num(1000.0001)));
        assert_eq!(p("-.25"), Some(Value::Num(-0.25)));
    }

    #[test]
    fn not_numbers() {
        for s in [
            "inf", "-inf", "nan", "NaN", "infinity", "1,00", "1,0000", ",5", "1__0", "_1", "1_",
            "1._5", "1e", "1e+", "e5", ".", "-", "+", "1.2.3", "1,000,00", "12a", "0x10", "1 000",
            "--1", "1e3.5",
        ] {
            assert_eq!(p(s), Some(Value::str(s)), "{s:?}");
        }
    }

    proptest! {
        #[test]
        fn ints_round_trip(i: i64) {
            prop_assert_eq!(p(&i.to_string()), Some(Value::Int(i)));
        }

        #[test]
        fn finite_floats_round_trip(f in proptest::num::f64::NORMAL | proptest::num::f64::SUBNORMAL | proptest::num::f64::ZERO) {
            let v = Value::Num(f);
            prop_assert_eq!(p(&v.edit_text()), Some(v));
        }

        #[test]
        fn any_string_round_trips_through_edit_text(s in ".*") {
            let v = Value::str(s.as_str());
            prop_assert_eq!(p(&v.edit_text()), Some(v));
        }

        #[test]
        fn never_panics(s in "\\PC*") {
            let _ = p(&s);
        }
    }
}
