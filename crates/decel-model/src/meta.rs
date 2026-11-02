// SPDX-License-Identifier: MIT OR Apache-2.0

//! Metatable sources, canonical form and content hashing (DESIGN §4.1–4.2).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Standard hooks and their parameter lists, in the editor's display order.
pub const HOOKS: &[(&str, &str)] = &[
    ("value", "self, c"),
    ("display", "self, c, v"),
    ("edit", "self, c, v"),
    ("parse", "self, c, text"),
    ("validate", "self, c, v"),
    ("tooltip", "self, c, v"),
    ("edit_hint", "self, c"),
];

/// Standard fields.
pub const FIELDS: &[&str] = &["kind", "unit", "style"];

const CANONICAL_HEADER: &str = "decel-meta/1\n";

const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemberKind {
    /// A literal or expression (`kind`, `unit`, `style`).
    Field,
    /// A standard hook with a fixed signature; the source is the body.
    Hook,
    /// A free-form expression, usually `function(self, ...) ... end`.
    Helper,
}

impl MemberKind {
    /// The kind a member with this name gets by default.
    pub fn for_name(name: &str) -> Self {
        if HOOKS.iter().any(|(h, _)| *h == name) {
            MemberKind::Hook
        } else if FIELDS.contains(&name) {
            MemberKind::Field
        } else {
            MemberKind::Helper
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            MemberKind::Field => "field",
            MemberKind::Hook => "hook",
            MemberKind::Helper => "helper",
        }
    }
}

impl FromStr for MemberKind {
    type Err = MetaParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "field" => Ok(MemberKind::Field),
            "hook" => Ok(MemberKind::Hook),
            "helper" => Ok(MemberKind::Helper),
            other => Err(MetaParseError::BadKind(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Member {
    pub name: String,
    pub kind: MemberKind,
    /// For hooks, the function body; otherwise a Lua expression.
    pub source: String,
}

impl Member {
    pub fn new(name: impl Into<String>, source: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            kind: MemberKind::for_name(&name),
            name,
            source: source.into(),
        }
    }

    /// The parameter list for a hook, if this is a standard hook.
    pub fn signature(&self) -> Option<&'static str> {
        HOOKS
            .iter()
            .find(|(h, _)| *h == self.name)
            .map(|(_, params)| *params)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MetaParseError {
    #[error("not a decel metatable (missing header)")]
    MissingHeader,
    #[error("malformed member record")]
    Malformed,
    #[error("unknown member kind {0:?}")]
    BadKind(String),
    #[error("duplicate member {0:?}")]
    Duplicate(String),
    #[error("{0:?} is not a valid member name")]
    BadName(String),
    #[error("{0:?} is a standard hook and must have kind hook")]
    HookKind(String),
}

/// An ordered set of metatable members.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MetaSource {
    pub members: Vec<Member>,
}

/// 16-byte truncated BLAKE3 hash of a metatable's canonical source.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MetaHash(pub [u8; 16]);

impl MetaHash {
    pub fn from_slice(b: &[u8]) -> Option<Self> {
        b.try_into().ok().map(MetaHash)
    }
}

impl fmt::Display for MetaHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for MetaHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MetaHash({self})")
    }
}

/// Normalises member source: `\n` line endings, trailing whitespace trimmed
/// from every line and from the end, and exactly one trailing newline.
pub fn normalise_source(src: &str) -> String {
    let unified = src.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(unified.len() + 1);
    for line in unified.lines() {
        out.push_str(line.trim_end());
        out.push('\n');
    }
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    out.push('\n');
    out
}

fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !LUA_KEYWORDS.contains(&name)
}

impl MetaSource {
    /// Builds a metatable from members, checking names.
    pub fn new(members: Vec<Member>) -> Result<Self, MetaParseError> {
        let src = Self { members };
        src.check()?;
        Ok(src)
    }

    /// Convenience builder: adds a member whose kind is inferred from its name.
    pub fn with(mut self, name: &str, source: &str) -> Self {
        self.set(Member::new(name, source));
        self
    }

    /// Adds or replaces a member by name.
    pub fn set(&mut self, member: Member) {
        match self.members.iter_mut().find(|m| m.name == member.name) {
            Some(m) => *m = member,
            None => self.members.push(member),
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<Member> {
        let i = self.members.iter().position(|m| m.name == name)?;
        Some(self.members.remove(i))
    }

    pub fn get(&self, name: &str) -> Option<&Member> {
        self.members.iter().find(|m| m.name == name)
    }

    pub fn check(&self) -> Result<(), MetaParseError> {
        let mut seen = std::collections::HashSet::new();
        for m in &self.members {
            if !is_valid_name(&m.name) {
                return Err(MetaParseError::BadName(m.name.clone()));
            }
            if !seen.insert(m.name.as_str()) {
                return Err(MetaParseError::Duplicate(m.name.clone()));
            }
            if m.signature().is_some() && m.kind != MemberKind::Hook {
                return Err(MetaParseError::HookKind(m.name.clone()));
            }
        }
        Ok(())
    }

    /// The canonical serialisation: members sorted by name with normalised
    /// source.
    pub fn canonical(&self) -> String {
        let mut members: Vec<&Member> = self.members.iter().collect();
        members.sort_by(|a, b| a.name.cmp(&b.name));
        let mut out = String::from(CANONICAL_HEADER);
        for m in members {
            let src = normalise_source(&m.source);
            out.push_str(&format!(
                "{}\t{}\t{}\n{}\n",
                m.name,
                m.kind.as_str(),
                src.len(),
                src
            ));
        }
        out
    }

    pub fn hash(&self) -> MetaHash {
        let digest = blake3::hash(self.canonical().as_bytes());
        let mut out = [0u8; 16];
        out.copy_from_slice(&digest.as_bytes()[..16]);
        MetaHash(out)
    }

    /// Parses a canonical serialisation back into members (sorted by name).
    pub fn from_canonical(text: &str) -> Result<Self, MetaParseError> {
        let mut rest = text
            .strip_prefix(CANONICAL_HEADER)
            .ok_or(MetaParseError::MissingHeader)?;
        let mut members = Vec::new();
        while !rest.is_empty() {
            let (header, after) = rest.split_once('\n').ok_or(MetaParseError::Malformed)?;
            let mut parts = header.split('\t');
            let (Some(name), Some(kind), Some(len), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                return Err(MetaParseError::Malformed);
            };
            let len: usize = len.parse().map_err(|_| MetaParseError::Malformed)?;
            let source = after.get(..len).ok_or(MetaParseError::Malformed)?;
            rest = after
                .get(len..)
                .and_then(|r| r.strip_prefix('\n'))
                .ok_or(MetaParseError::Malformed)?;
            members.push(Member {
                name: name.to_string(),
                kind: kind.parse()?,
                source: source.strip_suffix('\n').unwrap_or(source).to_string(),
            });
        }
        Self::new(members)
    }

    /// The `kind` field's literal string value, if it is a simple string literal.
    pub fn kind_label(&self) -> Option<String> {
        literal_string(&self.get("kind")?.source)
    }

    pub fn unit_label(&self) -> Option<String> {
        literal_string(&self.get("unit")?.source)
    }
}

/// Extracts the contents of a simple `"..."` or `'...'` literal.
fn literal_string(src: &str) -> Option<String> {
    let s = src.trim();
    let quote = s.chars().next()?;
    if (quote == '"' || quote == '\'') && s.len() >= 2 && s.ends_with(quote) {
        let inner = &s[1..s.len() - 1];
        if !inner.contains(quote) && !inner.contains('\\') {
            return Some(inner.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn latitude() -> MetaSource {
        MetaSource::default()
            .with("kind", "\"latitude\"")
            .with("unit", "\"deg\"")
            .with("display", "return fmt.dms(v, \"lat\", 1)")
    }

    #[test]
    fn same_source_same_hash_regardless_of_order() {
        let a = latitude();
        let b = MetaSource::default()
            .with("display", "return fmt.dms(v, \"lat\", 1)")
            .with("unit", "\"deg\"")
            .with("kind", "\"latitude\"");
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn trailing_whitespace_does_not_change_hash() {
        let a = latitude();
        let b = MetaSource::default()
            .with("kind", "\"latitude\"   ")
            .with("unit", "\"deg\"\r\n\r\n")
            .with("display", "return fmt.dms(v, \"lat\", 1)\t\n");
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn any_other_change_changes_hash() {
        let base = latitude().hash();
        assert_ne!(base, latitude().with("kind", "\"Latitude\"").hash());
        assert_ne!(base, latitude().with("kind", " \"latitude\"").hash());
        assert_ne!(base, latitude().with("tooltip", "return nil").hash());
        let mut removed = latitude();
        removed.remove("unit");
        assert_ne!(base, removed.hash());
        // Kind is part of identity.
        let mut helper = latitude();
        helper.set(Member {
            name: "kind".into(),
            kind: MemberKind::Helper,
            source: "\"latitude\"".into(),
        });
        assert_ne!(base, helper.hash());
    }

    #[test]
    fn canonical_round_trips() {
        let src = latitude().with("helper", "function(self)\n  return 1\nend");
        let back = MetaSource::from_canonical(&src.canonical()).unwrap();
        assert_eq!(back.hash(), src.hash());
        assert_eq!(
            back.get("helper").unwrap().source,
            "function(self)\n  return 1\nend"
        );
    }

    #[test]
    fn canonical_handles_tabs_and_newlines_in_source() {
        let src = MetaSource::default().with("value", "local s = \"a\\tb\"\n\n\treturn s");
        let back = MetaSource::from_canonical(&src.canonical()).unwrap();
        assert_eq!(back.canonical(), src.canonical());
    }

    #[test]
    fn rejects_bad_names_and_duplicates() {
        assert!(MetaSource::new(vec![Member::new("1x", "1")]).is_err());
        assert!(MetaSource::new(vec![Member::new("end", "1")]).is_err());
        assert!(MetaSource::new(vec![Member::new("a", "1"), Member::new("a", "2")]).is_err());
        assert!(
            MetaSource::new(vec![Member {
                name: "value".into(),
                kind: MemberKind::Helper,
                source: "1".into()
            }])
            .is_err()
        );
    }

    #[test]
    fn labels() {
        assert_eq!(latitude().kind_label().as_deref(), Some("latitude"));
        assert_eq!(latitude().unit_label().as_deref(), Some("deg"));
        assert_eq!(
            MetaSource::default().with("kind", "x .. y").kind_label(),
            None
        );
    }

    #[test]
    fn normalise() {
        assert_eq!(normalise_source("a  \r\nb\t\n\n\n"), "a\nb\n");
        assert_eq!(normalise_source(""), "\n");
    }
}
