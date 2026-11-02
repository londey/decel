// SPDX-License-Identifier: MIT OR Apache-2.0

//! What the UI paints for a cell (DESIGN §4.4).

use std::sync::Arc;

use decel_model::{CellError, MetaHash};
use mlua::{Table, Value as LuaValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Centre,
    Right,
}

impl Align {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "left" => Some(Align::Left),
            "centre" | "center" => Some(Align::Centre),
            "right" => Some(Align::Right),
            _ => None,
        }
    }
}

/// Style overrides from a `display` table or the `style` field. `None`
/// means "not specified here".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CellStyle {
    pub fg: Option<(u8, u8, u8)>,
    pub bg: Option<(u8, u8, u8)>,
    pub align: Option<Align>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub mono: Option<bool>,
}

impl CellStyle {
    /// Fills unspecified fields from `fallback`.
    pub fn or(self, fallback: &CellStyle) -> CellStyle {
        CellStyle {
            fg: self.fg.or(fallback.fg),
            bg: self.bg.or(fallback.bg),
            align: self.align.or(fallback.align),
            bold: self.bold.or(fallback.bold),
            italic: self.italic.or(fallback.italic),
            mono: self.mono.or(fallback.mono),
        }
    }

    /// Reads style keys from a Lua table. Unknown or malformed keys are
    /// ignored.
    pub fn from_table(t: &Table) -> CellStyle {
        let colour = |key: &str| -> Option<(u8, u8, u8)> {
            match t.get::<LuaValue>(key).ok()? {
                LuaValue::String(s) => crate::stdlib::color::parse_rgb(&s.to_string_lossy()),
                _ => None,
            }
        };
        let flag = |key: &str| -> Option<bool> {
            match t.get::<LuaValue>(key).ok()? {
                LuaValue::Boolean(b) => Some(b),
                _ => None,
            }
        };
        let align = match t.get::<LuaValue>("align") {
            Ok(LuaValue::String(s)) => Align::parse(&s.to_string_lossy()),
            _ => None,
        };
        CellStyle {
            fg: colour("fg"),
            bg: colour("bg"),
            align,
            bold: flag("bold"),
            italic: flag("italic"),
            mono: flag("mono"),
        }
    }
}

/// A fully resolved cell for painting.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayCell {
    pub text: String,
    pub style: CellStyle,
    /// The cell's final value is an error.
    pub error: Option<CellError>,
    /// A display hook failed; the default display is shown instead.
    pub display_error: Option<CellError>,
    /// Validation of a computed value failed.
    pub warning: Option<Arc<str>>,
    pub hash: Option<MetaHash>,
    pub kind: Option<Arc<str>>,
    pub unit: Option<Arc<str>>,
}

impl DisplayCell {
    pub fn align(&self) -> Align {
        self.style.align.unwrap_or_default()
    }

    /// Whether the cell shows an error or warning corner marker.
    pub fn has_marker(&self) -> bool {
        self.error.is_some() || self.display_error.is_some() || self.warning.is_some()
    }
}
