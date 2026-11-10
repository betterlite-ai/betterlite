#![cfg(test)]
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::str;

use rstest::rstest;

use super::{HeadersError, HeadersInput, headers_dictionary};

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::absent(HeadersInput::None, &[])]
#[case::empty_text(HeadersInput::Text(""), &[])]
#[case::plain(HeadersInput::Text("a=b,c=d"), &[("a", "b"), ("c", "d")])]
#[case::first_equals(HeadersInput::Text("authorization=Basic dG9rZW4="), &[("authorization", "Basic dG9rZW4=")])]
#[case::no_trimming(HeadersInput::Text(" a = b "), &[(" a ", " b ")])]
#[case::no_lowercasing(HeadersInput::Text("X-Key=Value"), &[("X-Key", "Value")])]
#[case::no_percent_decoding(HeadersInput::Text("x-token=Basic%20dG9rZW4%3D"), &[("x-token", "Basic%20dG9rZW4%3D")])]
#[case::later_duplicate_wins(HeadersInput::Text("a=1,a=2"), &[("a", "2")])]
#[case::empty_name_kept(HeadersInput::Text("=v"), &[("", "v")])]
#[case::empty_value_kept(HeadersInput::Text("a="), &[("a", "")])]
fn text_headers_split_on_commas_and_first_equals(
    #[case] input: HeadersInput<'_>,
    #[case] expected: &[(&str, &str)],
) {
    assert_eq!(headers_dictionary(input), Ok(map(expected)));
}

#[test]
fn an_empty_map_gives_an_empty_map() {
    assert_eq!(
        headers_dictionary(HeadersInput::Map(&BTreeMap::new())),
        Ok(BTreeMap::new())
    );
}

#[test]
fn map_headers_are_copied() {
    let source = map(&[("x-key", "secret"), ("X-Other", "1")]);
    assert_eq!(
        headers_dictionary(HeadersInput::Map(&source)),
        Ok(source.clone())
    );
}

#[rstest]
#[case::missing_equals(HeadersInput::Text("oops"), 1)]
#[case::second_part(HeadersInput::Text("a=b,oops"), 2)]
#[case::trailing_comma(HeadersInput::Text("a=b,"), 2)]
#[case::empty_middle_part(HeadersInput::Text("a=b,,c=d"), 2)]
#[case::colon_separator(HeadersInput::Text("Authorization: Bearer"), 1)]
fn text_headers_report_the_position_of_a_part_without_equals(
    #[case] input: HeadersInput<'_>,
    #[case] position: usize,
) {
    assert_eq!(
        headers_dictionary(input),
        Err(HeadersError::MissingSeparator { position })
    );
}

#[test]
fn a_missing_separator_error_holds_no_header_text() {
    let error = headers_dictionary(HeadersInput::Text("x-api-key=sk-live-secret,broken-entry"))
        .expect_err("the second entry has no equals sign");
    let rendered = error.to_string();
    assert_eq!(error, HeadersError::MissingSeparator { position: 2 });
    assert!(!rendered.contains("sk-live-secret"));
    assert!(!rendered.contains("broken-entry"));
    assert!(rendered.contains('2'));
}

#[rstest]
fn replays_the_legacy_headers_corpus(
    #[files("fuzz/corpus/legacy_headers_dictionary/*")] path: PathBuf,
) {
    let data = fs::read(&path).expect("corpus file");
    let Ok(text) = str::from_utf8(&data) else {
        return;
    };
    drop(headers_dictionary(HeadersInput::Text(text)));
}
