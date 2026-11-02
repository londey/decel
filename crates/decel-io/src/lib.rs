// SPDX-License-Identifier: MIT OR Apache-2.0

//! CSV import and export.
//!
//! This crate deals only in strings: [`read`] turns bytes into a [`Table`] of
//! text fields and [`write`] turns rows of text fields into CSV text. Typing
//! the values is the engine's job.
//!
//! # Behaviour notes
//!
//! - **Encoding**: [`decode`] strips a UTF-8 BOM. With [`Encoding::Auto`] the
//!   input is UTF-8 if it is valid UTF-8 (or starts with a BOM, in which case
//!   invalid sequences are replaced with U+FFFD), otherwise Windows-1252.
//! - **Blank lines**: a line that is completely empty *between* two records is
//!   one record with a single empty field (`[""]`). Blank lines before the
//!   first record and after the last record are ignored, so a trailing newline
//!   (or several) at EOF never produces extra records.
//! - **Ragged rows**: each record keeps its own field count; nothing is padded.
//! - **Header**: with [`ImportOptions::header`], the first record becomes
//!   [`Table::header`]. An empty input gives `header: None` and no rows.
//! - **Writing**: a row with zero fields is written as an empty line (which
//!   reads back as `[""]`); a row consisting of a single empty field is
//!   written as `""` so other tools do not skip it.

/// Text encoding of a CSV file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    /// Detect: valid UTF-8 → UTF-8, otherwise Windows-1252.
    #[default]
    Auto,
    /// UTF-8 (invalid sequences are replaced with U+FFFD).
    Utf8,
    /// Windows-1252 (a superset of ISO-8859-1 printable characters).
    Windows1252,
}

/// Options for [`read`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOptions {
    /// Field delimiter; `None` = auto-detect with [`detect_delimiter`].
    pub delimiter: Option<u8>,
    /// The first record is a header row.
    pub header: bool,
    /// Input encoding.
    pub encoding: Encoding,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            delimiter: None,
            header: false,
            encoding: Encoding::Auto,
        }
    }
}

/// A parsed CSV file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    /// The header row, if [`ImportOptions::header`] was set and the input was
    /// not empty.
    pub header: Option<Vec<String>>,
    /// Data rows; ragged (each record keeps its own length).
    pub rows: Vec<Vec<String>>,
    /// The delimiter actually used.
    pub delimiter: u8,
    /// The encoding actually used ([`Encoding::Utf8`] or
    /// [`Encoding::Windows1252`]).
    pub encoding: Encoding,
}

/// Errors from reading or writing CSV.
#[derive(Debug, thiserror::Error)]
pub enum CsvError {
    /// The CSV text could not be parsed.
    #[error("CSV parse error on line {line}: {message}")]
    Parse {
        /// 1-based line number where the error was detected (0 if unknown).
        line: u64,
        /// Description of the problem.
        message: String,
    },
    /// The delimiter cannot be used (it must be ASCII and not `"`, CR or LF).
    #[error("invalid CSV delimiter {0:#04x}")]
    InvalidDelimiter(u8),
}

/// Quoting policy for [`write`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quote {
    /// Quote only fields containing the delimiter, `"`, CR or LF (and a row
    /// that is a single empty field).
    #[default]
    Necessary,
    /// Quote every field.
    Always,
    /// Quote every field that is not numeric (and any field that needs it).
    NonNumeric,
}

/// Line terminator for [`write`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// `\n`
    #[default]
    Lf,
    /// `\r\n`
    CrLf,
}

/// Options for [`write`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportOptions {
    /// Field delimiter.
    pub delimiter: u8,
    /// Quoting policy.
    pub quote: Quote,
    /// Line terminator, written after every row (including the last).
    pub line_ending: LineEnding,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            quote: Quote::Necessary,
            line_ending: LineEnding::Lf,
        }
    }
}

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Windows-1252 code points for bytes 0x80..=0x9F. The five undefined bytes
/// (0x81, 0x8D, 0x8F, 0x90, 0x9D) map to the C1 control with the same value,
/// as in the WHATWG Encoding Standard.
const CP1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

fn decode_cp1252(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => CP1252_HIGH[usize::from(b - 0x80)],
            _ => char::from(b),
        })
        .collect()
}

/// Decodes bytes: strips a UTF-8 BOM; Auto = valid UTF-8 → Utf8, otherwise
/// Windows-1252. Returns the text and the encoding actually used.
///
/// With [`Encoding::Auto`], input that starts with a BOM is always treated as
/// UTF-8 (invalid sequences become U+FFFD). The BOM is stripped whatever the
/// requested encoding.
pub fn decode(bytes: &[u8], encoding: Encoding) -> (String, Encoding) {
    let (body, had_bom) = match bytes.strip_prefix(UTF8_BOM) {
        Some(rest) => (rest, true),
        None => (bytes, false),
    };
    match encoding {
        Encoding::Windows1252 => (decode_cp1252(body), Encoding::Windows1252),
        Encoding::Utf8 => (String::from_utf8_lossy(body).into_owned(), Encoding::Utf8),
        Encoding::Auto => match std::str::from_utf8(body) {
            Ok(s) => (s.to_owned(), Encoding::Utf8),
            Err(_) if had_bom => (String::from_utf8_lossy(body).into_owned(), Encoding::Utf8),
            Err(_) => (decode_cp1252(body), Encoding::Windows1252),
        },
    }
}

/// Candidate delimiters, in tie-break priority order.
const CANDIDATES: [u8; 4] = *b"\t;|,";
const SAMPLE_RECORDS: usize = 50;

/// Picks the most likely delimiter among `,` `;` `\t` `|` by consistency of
/// field counts over the first ~50 records (respecting quotes). Defaults to `,`.
///
/// A candidate is eligible only if the most common field count it produces is
/// at least 2. Eligible candidates are ranked by the fraction of sampled
/// records that have that most common count; ties go to tab, then `;`, then
/// `|`, then `,` — so a European file like `1,5;2,25` (consistent under both
/// `;` and `,`) is read as semicolon-separated.
pub fn detect_delimiter(text: &str) -> u8 {
    let mut per_record: Vec<[usize; 4]> = Vec::new();
    let mut counts = [0usize; 4];
    let mut in_quotes = false;
    let mut record_has_content = false;
    let mut prev_cr = false;

    for &b in text.as_bytes() {
        let was_cr = prev_cr;
        prev_cr = false;
        if in_quotes {
            if b == b'"' {
                in_quotes = false;
            }
            continue;
        }
        match b {
            b'"' => {
                in_quotes = true;
                record_has_content = true;
            }
            b'\r' | b'\n' => {
                if b == b'\n' && was_cr {
                    continue;
                }
                prev_cr = b == b'\r';
                if record_has_content {
                    per_record.push(counts);
                    if per_record.len() >= SAMPLE_RECORDS {
                        break;
                    }
                }
                counts = [0; 4];
                record_has_content = false;
            }
            _ => {
                record_has_content = true;
                if let Some(i) = CANDIDATES.iter().position(|&c| c == b) {
                    counts[i] += 1;
                }
            }
        }
    }
    if record_has_content && per_record.len() < SAMPLE_RECORDS {
        per_record.push(counts);
    }
    if per_record.is_empty() {
        return b',';
    }

    // (consistency numerator, candidate index); higher numerator wins, then
    // lower index (priority order).
    let mut best: Option<(usize, usize)> = None;
    for i in 0..CANDIDATES.len() {
        let mut freq: Vec<(usize, usize)> = Vec::new(); // (delimiter count, records)
        for rec in &per_record {
            let n = rec[i];
            match freq.iter_mut().find(|(k, _)| *k == n) {
                Some(entry) => entry.1 += 1,
                None => freq.push((n, 1)),
            }
        }
        // Mode; ties go to the larger field count.
        let Some(&(mode, hits)) = freq.iter().max_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)))
        else {
            continue;
        };
        if mode == 0 {
            continue;
        }
        if best.is_none_or(|(h, _)| hits > h) {
            best = Some((hits, i));
        }
    }
    best.map_or(b',', |(_, i)| CANDIDATES[i])
}

fn check_delimiter(delimiter: u8) -> Result<(), CsvError> {
    if !delimiter.is_ascii() || matches!(delimiter, b'"' | b'\r' | b'\n') {
        Err(CsvError::InvalidDelimiter(delimiter))
    } else {
        Ok(())
    }
}

/// Number of line terminators (`\n`, `\r\n` or a lone `\r`) in `gap`.
fn count_line_breaks(gap: &[u8]) -> usize {
    let mut n = 0;
    let mut i = 0;
    while i < gap.len() {
        match gap[i] {
            b'\n' => n += 1,
            b'\r' => {
                n += 1;
                if gap.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    n
}

fn parse_error(err: &csv::Error) -> CsvError {
    let line = err.position().map_or(0, csv::Position::line);
    CsvError::Parse {
        line,
        message: err.to_string(),
    }
}

/// Reads CSV bytes into a [`Table`]. See the crate docs for the handling of
/// blank lines, BOMs and encodings.
pub fn read(bytes: &[u8], opts: &ImportOptions) -> Result<Table, CsvError> {
    let (text, encoding) = decode(bytes, opts.encoding);
    let delimiter = match opts.delimiter {
        Some(d) => {
            check_delimiter(d)?;
            d
        }
        None => detect_delimiter(&text),
    };

    let data = text.as_bytes();
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .quoting(true)
        .double_quote(true)
        .from_reader(data);

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut record = csv::StringRecord::new();
    let mut first = true;
    loop {
        match reader.read_record(&mut record) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => return Err(parse_error(&e)),
        }
        // The record's position is where this read started, i.e. just past the
        // previous record. The reader silently skips blank lines, so any line
        // terminators between there and the record's first byte are blank
        // lines (a record never starts with CR or LF).
        if !first && let Some(pos) = record.position() {
            let mut from = usize::try_from(pos.byte()).unwrap_or(data.len());
            // The previous read may have stopped between the CR and LF of a
            // CRLF terminator.
            if from > 0 && data[from - 1] == b'\r' && data.get(from) == Some(&b'\n') {
                from += 1;
            }
            let rest = data.get(from..).unwrap_or_default();
            let gap_len = rest
                .iter()
                .position(|&b| b != b'\r' && b != b'\n')
                .unwrap_or(rest.len());
            let blanks = count_line_breaks(&rest[..gap_len]);
            rows.extend(std::iter::repeat_n(vec![String::new()], blanks));
        }
        first = false;
        rows.push(record.iter().map(str::to_owned).collect());
    }

    let header = if opts.header && !rows.is_empty() {
        Some(rows.remove(0))
    } else {
        None
    };
    Ok(Table {
        header,
        rows,
        delimiter,
        encoding,
    })
}

/// True if `field` parses as an `f64` and is not infinity/NaN spelled as text.
fn is_numeric(field: &str) -> bool {
    if field.parse::<f64>().is_err() {
        return false;
    }
    let unsigned = field.trim_start_matches(['+', '-']);
    !(unsigned.eq_ignore_ascii_case("inf")
        || unsigned.eq_ignore_ascii_case("infinity")
        || unsigned.eq_ignore_ascii_case("nan"))
}

/// Writes rows (each a slice of fields) as CSV text (UTF-8, no BOM). Rows may
/// be ragged. With [`Quote::NonNumeric`], a field is "numeric" if it parses as
/// `f64` and is not inf/nan text. Every row, including the last, is followed by
/// the line ending.
pub fn write(rows: &[Vec<String>], opts: &ExportOptions) -> Result<String, CsvError> {
    check_delimiter(opts.delimiter)?;
    let delim = char::from(opts.delimiter);
    let eol = match opts.line_ending {
        LineEnding::Lf => "\n",
        LineEnding::CrLf => "\r\n",
    };
    let mut out = String::with_capacity(
        rows.iter()
            .map(|r| r.iter().map(|f| f.len() + 1).sum::<usize>() + 2)
            .sum(),
    );
    for row in rows {
        let lone_empty = row.len() == 1 && row[0].is_empty();
        for (i, field) in row.iter().enumerate() {
            if i > 0 {
                out.push(delim);
            }
            let necessary = lone_empty || field.contains([delim, '"', '\r', '\n']);
            let quote = match opts.quote {
                Quote::Necessary => necessary,
                Quote::Always => true,
                Quote::NonNumeric => necessary || !is_numeric(field),
            };
            if quote {
                out.push('"');
                for c in field.chars() {
                    if c == '"' {
                        out.push('"');
                    }
                    out.push(c);
                }
                out.push('"');
            } else {
                out.push_str(field);
            }
        }
        out.push_str(eol);
    }
    Ok(out)
}
