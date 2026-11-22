#![cfg(test)]

use std::io::{self, BufRead, Read};

use rstest::rstest;

use super::support::{member, object};
use crate::api::file_batch_guardrails::error::BatchGuardrailError;
use crate::api::file_batch_guardrails::lines::{RawLine, iter_lines, iter_records};
use crate::api::file_batch_guardrails::record_json::RecordJson;
use crate::api::file_batch_guardrails::record_json::error::RecordJsonError;
use crate::api::file_batch_guardrails::record_json::text::RecordText;
use crate::api::file_batch_guardrails::records::ParsedRecord;

const FIRST: &[u8] = b"\xef\xbb\xbf{\"body\":{\"messages\":[{\"content\":\"\\ud800\"}]}}\n";
const THIRD: &[u8] = b"{\"body\":{\"messages\":[{\"content\":\"ok\"}]}}\n";
const SAMPLE: &[u8] = b"\xef\xbb\xbf{\"body\":{\"messages\":[{\"content\":\"\\ud800\"}]}}\n\n{\"body\":{\"messages\":[{\"content\":\"ok\"}]}}\n";

fn records(source: &[u8]) -> Vec<ParsedRecord> {
    iter_records(source).collect::<Result<_, _>>().unwrap()
}

fn content(record: &ParsedRecord) -> &RecordJson {
    let body = member(&record.payload, "body").as_object().unwrap();
    let RecordJson::List(messages) = member(body, "messages") else {
        panic!("messages is a list");
    };
    member(messages[0].as_object().unwrap(), "content")
}

#[test]
fn yields_untouched_bytes_at_their_physical_lines() {
    let lines: Vec<RawLine> = iter_lines(SAMPLE).collect::<Result<_, _>>().unwrap();
    assert_eq!(
        lines,
        vec![
            RawLine {
                line_number: 1,
                bytes: FIRST.to_vec(),
            },
            RawLine {
                line_number: 3,
                bytes: THIRD.to_vec(),
            },
        ]
    );
}

#[test]
fn parses_each_record_without_decoding_it_first() {
    let parsed = records(SAMPLE);
    assert_eq!(parsed.len(), 2);
    assert_eq!((parsed[0].line_number, parsed[1].line_number), (1, 3));
    let lone = RecordText::from_code_points(&[0xD800]);
    assert_eq!(content(&parsed[0]), &RecordJson::Text(lone));
    assert_eq!(
        content(&parsed[1]),
        &RecordJson::Text(RecordText::from("ok"))
    );
}

#[test]
fn skips_every_whitespace_python_strips() {
    let source = b" \t\r\x0b\x0c\n{\"body\":{}}\n\n\x0b\n{\"body\":{\"input\":null}}";
    let parsed = records(source);
    assert_eq!((parsed[0].line_number, parsed[1].line_number), (2, 5));
    assert_eq!(parsed[1].payload, object(r#"{"body":{"input":null}}"#));
}

#[test]
fn a_line_with_other_control_bytes_is_not_blank() {
    let lines: Vec<RawLine> = iter_lines(b"\x00\n".as_slice())
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(lines.len(), 1);
    let failure = iter_records(b"\n\x00\n".as_slice())
        .next()
        .unwrap()
        .unwrap_err();
    assert!(matches!(
        failure,
        BatchGuardrailError::Record {
            line_number: 2,
            source: RecordJsonError::Syntax
        }
    ));
}

fn utf16(text: &str, bom: &[u8], to_bytes: fn(u16) -> [u8; 2]) -> Vec<u8> {
    bom.iter()
        .copied()
        .chain(text.encode_utf16().flat_map(to_bytes))
        .collect()
}

fn utf32(text: &str, bom: &[u8], to_bytes: fn(u32) -> [u8; 4]) -> Vec<u8> {
    let units = text
        .chars()
        .flat_map(|character| to_bytes(u32::from(character)));
    bom.iter().copied().chain(units).collect()
}

const WIDE: &str = "{\"custom_id\":\"\u{e9}\",\"body\":{\"prompt\":\"\u{1f600}\"}}";

#[rstest]
#[case::utf16_le_bom(utf16(WIDE, &[0xFF, 0xFE], u16::to_le_bytes))]
#[case::utf16_be_bom(utf16(WIDE, &[0xFE, 0xFF], u16::to_be_bytes))]
#[case::utf16_le_sniffed(utf16(WIDE, &[], u16::to_le_bytes))]
#[case::utf16_be_sniffed(utf16(WIDE, &[], u16::to_be_bytes))]
#[case::utf32_le_bom(utf32(WIDE, &[0xFF, 0xFE, 0x00, 0x00], u32::to_le_bytes))]
#[case::utf32_be_bom(utf32(WIDE, &[0x00, 0x00, 0xFE, 0xFF], u32::to_be_bytes))]
#[case::utf32_le_sniffed(utf32(WIDE, &[], u32::to_le_bytes))]
#[case::utf32_be_sniffed(utf32(WIDE, &[], u32::to_be_bytes))]
#[case::utf8_bom([b"\xef\xbb\xbf".as_slice(), WIDE.as_bytes()].concat())]
fn wide_records_stay_bytes_until_their_own_parse(#[case] line: Vec<u8>) {
    let lines: Vec<RawLine> = iter_lines(line.as_slice())
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(lines[0].bytes, line);
    assert_eq!(records(&line)[0].payload, object(WIDE));
}

struct Broken;

impl Read for Broken {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("disk gone"))
    }
}

impl BufRead for Broken {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        Err(io::Error::other("disk gone"))
    }

    fn consume(&mut self, _amount: usize) {}
}

#[test]
fn a_read_failure_ends_the_records_with_its_cause() {
    let failure = iter_records(Broken).next().unwrap().unwrap_err();
    assert!(matches!(failure, BatchGuardrailError::Read { .. }));
}

#[test]
fn an_empty_source_has_no_records() {
    assert_eq!(iter_records(b"".as_slice()).count(), 0);
}
