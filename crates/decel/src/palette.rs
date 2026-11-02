// SPDX-License-Identifier: MIT OR Apache-2.0

//! Metatable colours (DESIGN §8.3): 16 OKLCH hues, tuned separately for
//! light and dark themes. Index 16 and up repeat the hues and are drawn with
//! a hatch.

use gpui::{Hsla, Rgba};

pub const HUES: u32 = 16;

/// OKLCH → sRGB (0..=1 per channel, clamped).
fn oklch_to_srgb(l: f64, c: f64, h_deg: f64) -> (f64, f64, f64) {
    let h = h_deg.to_radians();
    let (a, b) = (c * h.cos(), c * h.sin());
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3;
    let g = -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3;
    let bl = -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701 * s3;
    let gamma = |x: f64| {
        let x = x.clamp(0.0, 1.0);
        if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        }
    };
    (gamma(r), gamma(g), gamma(bl))
}

/// The base colour for a metatable's colour index.
pub fn color(index: u32, dark: bool) -> Hsla {
    // Step through the hue wheel so consecutive indexes are far apart.
    let slot = (index % HUES) * 7 % HUES;
    let hue = 25.0 + f64::from(slot) * 360.0 / f64::from(HUES);
    let (l, c) = if dark { (0.62, 0.13) } else { (0.72, 0.13) };
    let (r, g, b) = oklch_to_srgb(l, c, hue);
    Rgba {
        r: r as f32,
        g: g as f32,
        b: b as f32,
        a: 1.0,
    }
    .into()
}

/// Whether this index repeats an earlier hue (drawn hatched).
pub fn hatched(index: u32) -> bool {
    index >= HUES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_distinct_hues() {
        let colors: Vec<Hsla> = (0..HUES).map(|i| color(i, false)).collect();
        for (i, a) in colors.iter().enumerate() {
            for b in &colors[i + 1..] {
                assert!((a.h - b.h).abs() > 0.01 || (a.l - b.l).abs() > 0.01);
            }
        }
        assert_eq!(color(3, true), color(3 + HUES, true));
        assert!(hatched(HUES) && !hatched(HUES - 1));
    }

    #[test]
    fn white_and_black() {
        let (r, g, b) = oklch_to_srgb(1.0, 0.0, 0.0);
        assert!((r - 1.0).abs() < 1e-3 && (g - 1.0).abs() < 1e-3 && (b - 1.0).abs() < 1e-3);
        let (r, g, b) = oklch_to_srgb(0.0, 0.0, 0.0);
        assert!(r.abs() < 1e-6 && g.abs() < 1e-6 && b.abs() < 1e-6);
    }
}
