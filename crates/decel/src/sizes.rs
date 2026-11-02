// SPDX-License-Identifier: MIT OR Apache-2.0

//! Row heights / column widths with O(log k) offset ↔ index lookups, where
//! k is the number of rows or columns that differ from the default.
//!
//! Offsets are `f64`: a million 24 px rows is 24 M px, beyond `f32`'s exact
//! integer range.

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct AxisSizes {
    default: f64,
    /// `(index, size, offset of index)` for each override, sorted by index.
    overrides: Vec<(u32, f64, f64)>,
}

impl AxisSizes {
    pub fn new(default: f64, overrides: &BTreeMap<u32, f64>) -> Self {
        let mut s = Self {
            default,
            overrides: Vec::with_capacity(overrides.len()),
        };
        let mut extra = 0.0;
        for (&i, &size) in overrides {
            let offset = f64::from(i) * default + extra;
            s.overrides.push((i, size, offset));
            extra += size - default;
        }
        s
    }

    pub fn default_size(&self) -> f64 {
        self.default
    }

    pub fn size(&self, idx: u32) -> f64 {
        match self.overrides.binary_search_by_key(&idx, |o| o.0) {
            Ok(k) => self.overrides[k].1,
            Err(_) => self.default,
        }
    }

    /// Distance from the start of index 0 to the start of `idx`.
    pub fn offset(&self, idx: u32) -> f64 {
        match self.overrides.binary_search_by_key(&idx, |o| o.0) {
            Ok(k) => self.overrides[k].2,
            Err(0) => f64::from(idx) * self.default,
            Err(k) => {
                let (i, size, off) = self.overrides[k - 1];
                off + size + f64::from(idx - i - 1) * self.default
            }
        }
    }

    /// The index whose span contains `offset` (clamped at 0).
    pub fn index_at(&self, offset: f64) -> u32 {
        if offset <= 0.0 {
            return 0;
        }
        // Last override starting at or before `offset`.
        let k = self.overrides.partition_point(|o| o.2 <= offset);
        let raw = if k == 0 {
            offset / self.default
        } else {
            let (i, size, off) = self.overrides[k - 1];
            if offset < off + size {
                return i;
            }
            f64::from(i) + 1.0 + (offset - off - size) / self.default
        };
        raw.floor().min(f64::from(u32::MAX)) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes(over: &[(u32, f64)]) -> AxisSizes {
        AxisSizes::new(10.0, &over.iter().copied().collect())
    }

    #[test]
    fn uniform() {
        let s = sizes(&[]);
        assert_eq!(s.offset(0), 0.0);
        assert_eq!(s.offset(5), 50.0);
        assert_eq!(s.index_at(49.9), 4);
        assert_eq!(s.index_at(50.0), 5);
        assert_eq!(s.index_at(-3.0), 0);
    }

    #[test]
    fn with_overrides() {
        let s = sizes(&[(2, 30.0), (5, 5.0)]);
        // 0:0 1:10 2:20(30) 3:50 4:60 5:70(5) 6:75 7:85
        let want = [0.0, 10.0, 20.0, 50.0, 60.0, 70.0, 75.0, 85.0];
        for (i, w) in want.iter().enumerate() {
            assert_eq!(s.offset(i as u32), *w, "offset {i}");
        }
        for (off, idx) in [
            (0.0, 0),
            (19.0, 1),
            (20.0, 2),
            (49.0, 2),
            (50.0, 3),
            (72.0, 5),
            (75.0, 6),
            (84.9, 6),
        ] {
            assert_eq!(s.index_at(off), idx, "index_at {off}");
        }
        assert_eq!(s.size(2), 30.0);
        assert_eq!(s.size(3), 10.0);
    }

    #[test]
    fn round_trips_at_a_million_rows() {
        let s = AxisSizes::new(24.0, &[(10, 50.0)].into_iter().collect());
        for idx in [0, 9, 10, 11, 999_999, 1_000_000] {
            assert_eq!(s.index_at(s.offset(idx)), idx);
            assert_eq!(s.index_at(s.offset(idx) + s.size(idx) - 0.01), idx);
        }
    }
}
