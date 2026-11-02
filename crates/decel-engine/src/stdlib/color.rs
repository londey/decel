// SPDX-License-Identifier: MIT OR Apache-2.0

//! Colour helpers exposed to Lua as `color.*` (DESIGN §4.5).
//!
//! Colours are lowercase `"#rrggbb"` strings. Functions taking colours accept anything
//! [`hex`] accepts.

fn to_channel(c: f64) -> u8 {
    if c.is_nan() {
        0
    } else {
        c.clamp(0.0, 255.0).round() as u8
    }
}

fn to_hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// `"#rrggbb"` from components clamped to 0..=255 and rounded (half away from zero).
/// NaN components count as 0.
pub fn rgb(r: f64, g: f64, b: f64) -> String {
    to_hex((to_channel(r), to_channel(g), to_channel(b)))
}

/// Parses `"#abc"`, `"abc"`, `"#AABBCC"` or `"aabbcc"` (surrounding whitespace ignored)
/// into its channels.
pub fn parse_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let nibble = |i: usize| s.get(i..=i).and_then(|d| u8::from_str_radix(d, 16).ok());
    let byte = |i: usize| s.get(i..i + 2).and_then(|d| u8::from_str_radix(d, 16).ok());
    match s.len() {
        3 => Some((nibble(0)? * 17, nibble(1)? * 17, nibble(2)? * 17)),
        6 => Some((byte(0)?, byte(2)?, byte(4)?)),
        _ => None,
    }
}

/// Normalises a colour to lowercase `"#rrggbb"`.
pub fn hex(s: &str) -> Option<String> {
    parse_rgb(s).map(to_hex)
}

/// Per-channel linear interpolation in sRGB; `t` is clamped to 0..=1 (NaN counts as 0).
pub fn lerp(a: &str, b: &str, t: f64) -> Option<String> {
    let (ar, ag, ab) = parse_rgb(a)?;
    let (br, bg, bb) = parse_rgb(b)?;
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    let mix = |x: u8, y: u8| f64::from(x) + (f64::from(y) - f64::from(x)) * t;
    Some(rgb(mix(ar, br), mix(ag, bg), mix(ab, bb)))
}

/// Heat-map colour: `lerp(a, b, (v - lo) / (hi - lo))`. `lo == hi` gives `a`; so does any
/// non-finite position (NaN `v`, infinite bounds).
pub fn scale(v: f64, lo: f64, hi: f64, a: &str, b: &str) -> Option<String> {
    // A NaN position (NaN input, inf/inf) is mapped to `a` by `lerp`; ±inf clamps to an end.
    let t = if lo == hi { 0.0 } else { (v - lo) / (hi - lo) };
    lerp(a, b, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: Option<String>) -> String {
        v.unwrap_or_default()
    }

    #[test]
    fn rgb_golden() {
        assert_eq!(rgb(255.0, 0.0, 128.0), "#ff0080");
        assert_eq!(rgb(0.0, 0.0, 0.0), "#000000");
        assert_eq!(rgb(-0.0, -10.0, 300.0), "#0000ff");
        assert_eq!(rgb(127.5, 127.4, 0.5), "#807f01");
        assert_eq!(rgb(f64::NAN, f64::INFINITY, f64::NEG_INFINITY), "#00ff00");
        assert_eq!(rgb(1e300, -1e300, 15.0), "#ff000f");
    }

    #[test]
    fn hex_golden() {
        assert_eq!(s(hex("#abc")), "#aabbcc");
        assert_eq!(s(hex("abc")), "#aabbcc");
        assert_eq!(s(hex("#AABBCC")), "#aabbcc");
        assert_eq!(s(hex("aabbcc")), "#aabbcc");
        assert_eq!(s(hex(" #0F0 ")), "#00ff00");
        for bad in [
            "",
            "#",
            "#ab",
            "#abcd",
            "#abcde",
            "#abcdefg",
            "ggg",
            "#12345g",
            "##abc",
            "#ａｂｃ",
            "+12345",
        ] {
            assert_eq!(hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn parse_rgb_golden() {
        assert_eq!(parse_rgb("#ff0080"), Some((255, 0, 128)));
        assert_eq!(parse_rgb("FFF"), Some((255, 255, 255)));
        assert_eq!(parse_rgb("#000"), Some((0, 0, 0)));
        assert_eq!(parse_rgb("nope"), None);
    }

    #[test]
    fn lerp_golden() {
        assert_eq!(s(lerp("#000000", "#ffffff", 0.5)), "#808080");
        assert_eq!(s(lerp("#000", "#fff", 0.0)), "#000000");
        assert_eq!(s(lerp("#000", "#fff", 1.0)), "#ffffff");
        assert_eq!(s(lerp("#000", "#fff", -1.0)), "#000000");
        assert_eq!(s(lerp("#000", "#fff", 2.0)), "#ffffff");
        assert_eq!(s(lerp("#000", "#fff", f64::NAN)), "#000000");
        assert_eq!(s(lerp("#000", "#fff", f64::INFINITY)), "#ffffff");
        assert_eq!(s(lerp("#ff0000", "#0000ff", 0.25)), "#bf0040");
        assert_eq!(lerp("#000", "nope", 0.5), None);
        assert_eq!(lerp("nope", "#000", 0.5), None);
    }

    #[test]
    fn scale_golden() {
        assert_eq!(s(scale(5.0, 0.0, 10.0, "#000", "#fff")), "#808080");
        assert_eq!(s(scale(0.0, 0.0, 10.0, "#000", "#fff")), "#000000");
        assert_eq!(s(scale(10.0, 0.0, 10.0, "#000", "#fff")), "#ffffff");
        assert_eq!(s(scale(-5.0, 0.0, 10.0, "#000", "#fff")), "#000000");
        assert_eq!(s(scale(50.0, 0.0, 10.0, "#000", "#fff")), "#ffffff");
        assert_eq!(s(scale(2.5, 10.0, 0.0, "#000", "#fff")), "#bfbfbf"); // reversed range
        assert_eq!(s(scale(3.0, 3.0, 3.0, "#123456", "#fff")), "#123456");
        assert_eq!(s(scale(f64::NAN, 0.0, 1.0, "#123", "#fff")), "#112233");
        assert_eq!(s(scale(f64::INFINITY, 0.0, 1.0, "#000", "#fff")), "#ffffff");
        assert_eq!(
            s(scale(f64::NEG_INFINITY, 0.0, 1.0, "#000", "#fff")),
            "#000000"
        );
        assert_eq!(s(scale(0.5, 0.0, f64::INFINITY, "#123", "#fff")), "#112233");
        assert_eq!(scale(0.5, 0.0, 1.0, "#000", "nope"), None);
    }
}
