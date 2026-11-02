// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::{Duration, Instant};

use decel_io::{
    CsvError, Encoding, ExportOptions, ImportOptions, LineEnding, Quote, decode, detect_delimiter,
    read, write,
};
use proptest::prelude::*;

fn rows(data: &[&[&str]]) -> Vec<Vec<String>> {
    data.iter()
        .map(|r| r.iter().map(|s| (*s).to_owned()).collect())
        .collect()
}

fn opts(delimiter: Option<u8>) -> ImportOptions {
    ImportOptions {
        delimiter,
        ..ImportOptions::default()
    }
}

// ---------------------------------------------------------------- round trip

fn field() -> impl Strategy<Value = String> {
    let chars = prop::sample::select(vec![
        'a', 'b', 'Z', '0', '9', '.', ' ', ',', ';', '\t', '|', '"', '\n', '\r', 'é', '€', '日',
        '🎉', '#', '\'',
    ]);
    prop::collection::vec(chars, 0..8).prop_map(|v| v.into_iter().collect())
}

fn table() -> impl Strategy<Value = Vec<Vec<String>>> {
    prop::collection::vec(prop::collection::vec(field(), 1..6), 0..8)
}

fn delimiter() -> impl Strategy<Value = u8> {
    prop::sample::select(vec![b',', b';', b'\t', b'|'])
}

fn quote() -> impl Strategy<Value = Quote> {
    prop::sample::select(vec![Quote::Necessary, Quote::Always, Quote::NonNumeric])
}

fn line_ending() -> impl Strategy<Value = LineEnding> {
    prop::sample::select(vec![LineEnding::Lf, LineEnding::CrLf])
}

proptest! {
    #[test]
    fn round_trip(data in table(), d in delimiter(), q in quote(), le in line_ending()) {
        let text = write(&data, &ExportOptions { delimiter: d, quote: q, line_ending: le }).unwrap();
        let t = read(text.as_bytes(), &opts(Some(d))).unwrap();
        prop_assert_eq!(t.rows, data);
        prop_assert_eq!(t.delimiter, d);
        prop_assert_eq!(t.encoding, Encoding::Utf8);
    }

    #[test]
    fn round_trip_with_header(header in prop::collection::vec(field(), 1..5), data in table()) {
        let mut all = vec![header.clone()];
        all.extend(data.iter().cloned());
        let text = write(&all, &ExportOptions::default()).unwrap();
        let t = read(text.as_bytes(), &ImportOptions { delimiter: Some(b','), header: true, ..ImportOptions::default() }).unwrap();
        prop_assert_eq!(t.header, Some(header));
        prop_assert_eq!(t.rows, data);
    }
}

#[test]
fn round_trip_awkward_fields() {
    let data = rows(&[
        &["a,b", "c;d", "say \"hi\"", "line1\nline2", "crlf\r\nend"],
        &[""],
        &["", "", ""],
        &["  lead", "trail  ", " both "],
        &["日本語", "naïve", "🎉"],
        &["\r", "\n", "\"", "\"\""],
    ]);
    for d in *b",;\t|" {
        let text = write(
            &data,
            &ExportOptions {
                delimiter: d,
                ..ExportOptions::default()
            },
        )
        .unwrap();
        assert_eq!(read(text.as_bytes(), &opts(Some(d))).unwrap().rows, data);
    }
}

// ------------------------------------------------------------------ reading

#[test]
fn reads_crlf_lf_and_multiline_quotes() {
    let t = read(b"a,b\r\nc,d\n\"x\r\ny\",z\n", &opts(None)).unwrap();
    assert_eq!(t.rows, rows(&[&["a", "b"], &["c", "d"], &["x\r\ny", "z"]]));
}

#[test]
fn trailing_newlines_do_not_add_records() {
    for input in [
        &b"a,b\n"[..],
        b"a,b\r\n",
        b"a,b",
        b"a,b\n\n",
        b"a,b\r\n\r\n\r\n",
    ] {
        let t = read(input, &opts(Some(b','))).unwrap();
        assert_eq!(t.rows, rows(&[&["a", "b"]]), "input {input:?}");
    }
}

#[test]
fn blank_line_in_middle_is_single_empty_field() {
    let t = read(b"a,b\n\nc\n", &opts(Some(b','))).unwrap();
    assert_eq!(t.rows, rows(&[&["a", "b"], &[""], &["c"]]));
    let t = read(b"a\r\n\r\n\r\nc\r\n", &opts(Some(b','))).unwrap();
    assert_eq!(t.rows, rows(&[&["a"], &[""], &[""], &["c"]]));
    // Leading blank lines are ignored.
    let t = read(b"\n\na\n", &opts(Some(b','))).unwrap();
    assert_eq!(t.rows, rows(&[&["a"]]));
}

#[test]
fn ragged_rows_keep_their_length() {
    let t = read(b"a,b,c\nd\ne,f\n", &opts(Some(b','))).unwrap();
    assert_eq!(t.rows, rows(&[&["a", "b", "c"], &["d"], &["e", "f"]]));
}

#[test]
fn header_option() {
    let input = b"name,age\nann,3\nbob,4\n";
    let t = read(
        input,
        &ImportOptions {
            header: true,
            ..ImportOptions::default()
        },
    )
    .unwrap();
    assert_eq!(t.header, Some(vec!["name".to_owned(), "age".to_owned()]));
    assert_eq!(t.rows, rows(&[&["ann", "3"], &["bob", "4"]]));

    let t = read(input, &ImportOptions::default()).unwrap();
    assert_eq!(t.header, None);
    assert_eq!(t.rows.len(), 3);

    let t = read(
        b"",
        &ImportOptions {
            header: true,
            ..ImportOptions::default()
        },
    )
    .unwrap();
    assert_eq!(t.header, None);
    assert!(t.rows.is_empty());
}

#[test]
fn empty_input() {
    let t = read(b"", &ImportOptions::default()).unwrap();
    assert!(t.rows.is_empty());
    assert_eq!(t.delimiter, b',');
    assert_eq!(t.encoding, Encoding::Utf8);
}

#[test]
fn invalid_delimiter_is_rejected() {
    assert!(matches!(
        read(b"a", &opts(Some(b'"'))),
        Err(CsvError::InvalidDelimiter(b'"'))
    ));
    assert!(matches!(
        write(
            &[],
            &ExportOptions {
                delimiter: b'\n',
                ..ExportOptions::default()
            }
        ),
        Err(CsvError::InvalidDelimiter(b'\n'))
    ));
    assert!(matches!(
        read(b"a", &opts(Some(0xE9))),
        Err(CsvError::InvalidDelimiter(0xE9))
    ));
}

// ------------------------------------------------------- delimiter detection

#[test]
fn detects_comma() {
    assert_eq!(detect_delimiter("a,b,c\n1,2,3\n4,5,6\n"), b',');
    // Semicolons inside quotes are ignored.
    assert_eq!(
        detect_delimiter("name,note\nann,\"x;y\"\nbob,\"p;q;r\"\n"),
        b','
    );
}

#[test]
fn detects_semicolon_with_decimal_commas() {
    let text = "a;b\n1,5;2,25\n3,75;4\n10;0,5\n";
    assert_eq!(detect_delimiter(text), b';');
    // Even when every row has the same number of decimal commas.
    assert_eq!(detect_delimiter("1,5;2,25\n3,5;4,75\n"), b';');
    let t = read(text.as_bytes(), &ImportOptions::default()).unwrap();
    assert_eq!(t.delimiter, b';');
    assert_eq!(t.rows[1], vec!["1,5".to_owned(), "2,25".to_owned()]);
}

#[test]
fn detects_tab() {
    assert_eq!(detect_delimiter("a\tb\tc\n1, 2\t3\t4\n5\t6\t7\n"), b'\t');
}

#[test]
fn detects_pipe() {
    assert_eq!(detect_delimiter("a|b|c\r\n1|2|3\r\n4|5|6\r\n"), b'|');
}

#[test]
fn detection_respects_quoted_newlines() {
    let text = "a,b\n\"multi\nline; with; semis\",x\nc,d\n";
    assert_eq!(detect_delimiter(text), b',');
}

#[test]
fn detection_defaults_to_comma() {
    assert_eq!(detect_delimiter(""), b',');
    assert_eq!(detect_delimiter("single\ncolumn\n"), b',');
}

// ----------------------------------------------------------------- encoding

#[test]
fn strips_utf8_bom() {
    let (s, e) = decode(b"\xEF\xBB\xBFa,b", Encoding::Auto);
    assert_eq!((s.as_str(), e), ("a,b", Encoding::Utf8));
    let t = read(b"\xEF\xBB\xBFx,y\n", &ImportOptions::default()).unwrap();
    assert_eq!(t.rows, rows(&[&["x", "y"]]));
}

#[test]
fn windows_1252_high_range() {
    let (s, e) = decode(b"\x80 \x93q\x94 \x85 \x99 \x9F", Encoding::Windows1252);
    assert_eq!(e, Encoding::Windows1252);
    assert_eq!(s, "€ \u{201C}q\u{201D} … ™ Ÿ");
}

#[test]
fn windows_1252_latin1_range() {
    let (s, _) = decode(b"caf\xE9 \xA0\xA9\xFF\xC0", Encoding::Windows1252);
    assert_eq!(s, "café \u{A0}©ÿÀ");
}

#[test]
fn auto_falls_back_to_windows_1252() {
    let (s, e) = decode(b"caf\xE9,\x80", Encoding::Auto);
    assert_eq!((s.as_str(), e), ("café,€", Encoding::Windows1252));
    let t = read(b"caf\xE9;\x80\n", &ImportOptions::default()).unwrap();
    assert_eq!(t.encoding, Encoding::Windows1252);
    assert_eq!(t.rows, rows(&[&["café", "€"]]));
}

#[test]
fn auto_prefers_valid_utf8() {
    let (s, e) = decode("café".as_bytes(), Encoding::Auto);
    assert_eq!((s.as_str(), e), ("café", Encoding::Utf8));
}

#[test]
fn explicit_utf8_is_lossy() {
    let (s, e) = decode(b"a\xFFb", Encoding::Utf8);
    assert_eq!((s.as_str(), e), ("a\u{FFFD}b", Encoding::Utf8));
}

// ------------------------------------------------------------------ writing

#[test]
fn write_quoting_styles() {
    let data = rows(&[&["1", "-2.5e3", "x", "", "inf", "NaN", "a,b"]]);
    let w = |quote| {
        write(
            &data,
            &ExportOptions {
                quote,
                ..ExportOptions::default()
            },
        )
        .unwrap()
    };
    assert_eq!(w(Quote::Necessary), "1,-2.5e3,x,,inf,NaN,\"a,b\"\n");
    assert_eq!(
        w(Quote::Always),
        "\"1\",\"-2.5e3\",\"x\",\"\",\"inf\",\"NaN\",\"a,b\"\n"
    );
    assert_eq!(
        w(Quote::NonNumeric),
        "1,-2.5e3,\"x\",\"\",\"inf\",\"NaN\",\"a,b\"\n"
    );
}

#[test]
fn write_line_endings_and_escaping() {
    let data = rows(&[&["say \"hi\"", "b"], &[""], &["c"]]);
    let out = write(
        &data,
        &ExportOptions {
            line_ending: LineEnding::CrLf,
            ..ExportOptions::default()
        },
    )
    .unwrap();
    assert_eq!(out, "\"say \"\"hi\"\"\",b\r\n\"\"\r\nc\r\n");
}

// -------------------------------------------------------------- performance

fn big_csv(n_rows: usize) -> Vec<u8> {
    let data: Vec<Vec<String>> = (0..n_rows)
        .map(|r| {
            (0..10)
                .map(|c| match c % 3 {
                    0 => format!("{}", r * 10 + c),
                    1 => format!("text {r} {c}"),
                    _ => format!("\"q\",{r}"),
                })
                .collect()
        })
        .collect();
    write(&data, &ExportOptions::default())
        .unwrap()
        .into_bytes()
}

#[test]
fn reads_moderate_file() {
    let bytes = big_csv(5_000);
    let t = read(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(t.delimiter, b',');
    assert_eq!(t.rows.len(), 5_000);
    assert!(t.rows.iter().all(|r| r.len() == 10));
    assert_eq!(t.rows[4_999][2], "\"q\",4999");
}

#[test]
#[ignore = "performance smoke test; run with --release -- --ignored"]
fn reads_100k_rows_quickly() {
    let bytes = big_csv(100_000);
    let start = Instant::now();
    let t = read(&bytes, &ImportOptions::default()).unwrap();
    let elapsed = start.elapsed();
    assert_eq!(t.rows.len(), 100_000);
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
}
