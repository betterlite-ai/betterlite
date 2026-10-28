#![cfg(test)]

use std::fs;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use rstest::rstest;

use super::chunk::Chunk;
use super::{SseParser, event_end};

#[rstest]
#[case::lf(b"data: first\n\nrest", Some(13))]
#[case::crlf(b"data: first\r\n\r\nrest", Some(15))]
#[case::incomplete(b"data: first\n", None)]
#[case::empty(b"", None)]
#[case::cr(b"data: first\r\r", Some(13))]
#[case::earliest(b"data: first\r\n\r\ndata: second\n\n", Some(15))]
fn stream_event_boundary(#[case] bytes: &[u8], #[case] expected: Option<usize>) {
    assert_eq!(event_end(bytes).map(NonZeroUsize::get), expected);
}

#[rstest]
#[case::lf(b"data: first\n\n", vec![b"first".to_vec()])]
#[case::crlf(b"data: first\r\n\r\n", vec![b"first".to_vec()])]
#[case::cr(b"data: first\r\r", vec![b"first".to_vec()])]
#[case::multiple(b"data: first\n\ndata: second\n\n", vec![b"first".to_vec(), b"second".to_vec()])]
#[case::comments(b": keepalive\n\nevent: ping\n\ndata: value\n\n", vec![b"value".to_vec()])]
#[case::multiline(b"data: first\ndata:second\n\n", vec![b"first\nsecond".to_vec()])]
#[case::spaces(b"data:  space\n\n", vec![b" space".to_vec()])]
fn stream_parses_events(#[case] input: &[u8], #[case] expected: Vec<Vec<u8>>) {
    let mut parser = SseParser::default();
    assert_eq!(parser.push(input).unwrap(), expected);
    assert!(parser.finish().is_empty());
}

#[test]
fn stream_parses_split_event() {
    let mut parser = SseParser::default();
    assert!(parser.push(b"data: {\"x\":").unwrap().is_empty());
    assert_eq!(parser.push(b"1}\n\n").unwrap(), vec![b"{\"x\":1}".to_vec()]);
}

#[rstest]
#[case::empty(b"", &[])]
#[case::comment(b": ping", &[])]
#[case::data(b"data: tail", &["tail"])]
#[case::multiline(b"data: a\ndata:b", &["a\nb"])]
fn stream_finish_drains_once(#[case] input: &[u8], #[case] expected: &[&str]) {
    let mut parser = SseParser::default();
    assert!(parser.push(input).unwrap().is_empty());
    assert_eq!(
        parser.finish(),
        expected
            .iter()
            .map(|text| text.as_bytes())
            .collect::<Vec<_>>()
    );
    assert!(parser.finish().is_empty());
    assert_eq!(
        parser.push(b"data: next\n\n").unwrap(),
        vec![b"next".to_vec()]
    );
}

#[rstest]
fn replays_the_sse_events_corpus(#[files("fuzz/corpus/sse_events/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let mut parser = SseParser::default();
    for chunk in bytes.chunks(1) {
        if let Ok(events) = parser.push(chunk) {
            for event in events {
                let _mapped = Chunk::parse(&event, "corpus", true);
            }
        }
    }
    let _remainder = parser.finish();
}

#[test]
fn stream_rejects_oversized_event() {
    let mut parser = SseParser::default();
    let bytes = vec![b'x'; super::MAX_BUFFER_BYTES];
    assert!(parser.push(&bytes).unwrap().is_empty());
    parser.push(b"x").unwrap_err();
}
