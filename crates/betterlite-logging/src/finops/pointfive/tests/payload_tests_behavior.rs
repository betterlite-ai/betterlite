#![cfg(test)]

use std::collections::BTreeMap;
use std::io::Read as _;

use flate2::read::GzDecoder;
use rstest::rstest;
use serde::Serialize;

use super::payload::{PayloadError, chunk_lines, encode_lines, serialize_records};

#[derive(Serialize)]
struct Record {
    id: &'static str,
    value: u32,
}

fn lines(contents: &[&str]) -> Vec<String> {
    contents.iter().map(|line| (*line).to_owned()).collect()
}

fn as_owned(chunks: &[Vec<&str>]) -> Vec<Vec<String>> {
    chunks
        .iter()
        .map(|chunk| chunk.iter().map(|line| (*line).to_owned()).collect())
        .collect()
}

fn decode(body: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    GzDecoder::new(body)
        .read_to_end(&mut decoded)
        .expect("a gzip stream");
    decoded
}

#[test]
fn each_record_becomes_one_json_line() {
    let records = [Record { id: "a", value: 1 }, Record { id: "b", value: 2 }];
    let encoded = serialize_records(&records).expect("the records serialize");
    assert_eq!(
        encoded,
        vec![
            r#"{"id":"a","value":1}"#.to_owned(),
            r#"{"id":"b","value":2}"#.to_owned()
        ]
    );
}

#[test]
fn no_records_make_no_lines() {
    let records: [Record; 0] = [];
    assert!(
        serialize_records(&records)
            .expect("the records serialize")
            .is_empty()
    );
}

#[test]
fn an_unserializable_record_is_an_error() {
    let records = [BTreeMap::from([(Some("key"), 1_u32)])];
    let error = serialize_records(&records).expect_err("non-string keys are refused");
    assert!(matches!(error, PayloadError::Serialize(_)));
}

#[test]
fn lines_stay_inside_the_byte_bound() {
    let input = lines(&["aaaa", "bbbb", "cccc"]);
    let chunks = chunk_lines(&input, 10);
    assert_eq!(
        as_owned(&chunks),
        vec![
            vec!["aaaa".to_owned(), "bbbb".to_owned()],
            vec!["cccc".to_owned()]
        ]
    );
}

#[test]
fn a_line_that_exactly_fills_the_bound_stays_in_its_object() {
    let input = lines(&["aaaa"]);
    let chunks = chunk_lines(&input, 5);
    assert_eq!(as_owned(&chunks), vec![vec!["aaaa".to_owned()]]);
}

#[test]
fn a_line_past_the_bound_opens_a_new_object() {
    let input = lines(&["aaaa", "bbbb"]);
    let chunks = chunk_lines(&input, 5);
    assert_eq!(
        as_owned(&chunks),
        vec![vec!["aaaa".to_owned()], vec!["bbbb".to_owned()]]
    );
}

#[test]
fn an_oversized_line_becomes_its_own_object() {
    let long = "x".repeat(20);
    let input = lines(&[long.as_str(), "b"]);
    let chunks = chunk_lines(&input, 5);
    assert_eq!(as_owned(&chunks), vec![vec![long], vec!["b".to_owned()]]);
}

#[test]
fn the_newline_counts_toward_each_line_size() {
    let input = lines(&["abcd", "e"]);
    let chunks = chunk_lines(&input, 5);
    assert_eq!(
        as_owned(&chunks),
        vec![vec!["abcd".to_owned()], vec!["e".to_owned()]]
    );
}

#[test]
fn sizes_count_utf8_bytes_not_characters() {
    let input = lines(&["\u{e9}\u{e9}", "x"]);
    let chunks = chunk_lines(&input, 6);
    assert_eq!(
        as_owned(&chunks),
        vec![vec!["\u{e9}\u{e9}".to_owned()], vec!["x".to_owned()]]
    );
}

#[test]
fn an_empty_input_makes_no_objects() {
    assert!(chunk_lines(&[], 10).is_empty());
}

#[test]
fn a_zero_bound_puts_every_line_alone() {
    let input = lines(&["a", "b"]);
    let chunks = chunk_lines(&input, 0);
    assert_eq!(
        as_owned(&chunks),
        vec![vec!["a".to_owned()], vec!["b".to_owned()]]
    );
}

#[rstest]
#[case::two_lines(&["{\"a\":1}", "{\"b\":2}"], "{\"a\":1}\n{\"b\":2}")]
#[case::one_line(&["only"], "only")]
#[case::no_lines(&[], "")]
#[tokio::test]
async fn encode_lines_joins_ndjson_and_gzips_it(#[case] input: &[&str], #[case] expected: &str) {
    let encoded = encode_lines(input).await.expect("the batch compresses");
    assert_eq!(encoded.get(..2), Some([0x1f, 0x8b].as_slice()));
    assert_eq!(decode(&encoded), expected.as_bytes());
}
