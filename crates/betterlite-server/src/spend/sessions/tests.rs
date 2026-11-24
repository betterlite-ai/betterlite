#![cfg(test)]

mod cursor;
mod expansion;
mod grouped;
mod postgres;
mod query;
mod support;

use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use rstest::rstest;

use super::cursor::decode_session_cursor;
use super::query::decode_session_query;

// each corpus file names its parser on the first line: a raw query string or a cursor value
#[rstest]
fn replays_the_session_inputs_corpus(#[files("fuzz/corpus/session_inputs/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let mut parts = bytes.splitn(2, |byte| *byte == b'\n');
    let tag = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default();
    match tag {
        b"query" => {
            let text = from_utf8(rest).unwrap();
            drop(decode_session_query(Some(text)));
        }
        b"cursor" => {
            let text = from_utf8(rest).unwrap();
            drop(decode_session_cursor(Some(text)));
        }
        other => panic!("unknown corpus tag {other:?}"),
    }
}
