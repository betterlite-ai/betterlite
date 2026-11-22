#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::{LineSplitter, event_from_line};

fn split_lines(bytes: &[u8]) -> Vec<String> {
    let split = usize::from(bytes.first().copied().unwrap_or_default());
    let (head, tail) = bytes.split_at_checked(split).unwrap_or((bytes, &[]));
    let mut splitter = LineSplitter::default();
    let mut lines = splitter.push(head);
    lines.extend(splitter.push(tail));
    lines.extend(splitter.finish());
    lines
}

// the corpus split is the fuzz target's, and where the bytes are split must not change the lines
#[rstest]
fn replays_the_a2a_sse_lines_corpus(#[files("fuzz/corpus/a2a_sse_lines/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let lines = split_lines(&bytes);
    let mut whole = LineSplitter::default();
    let mut unsplit = whole.push(&bytes);
    unsplit.extend(whole.finish());
    assert_eq!(lines, unsplit);
    drop(
        lines
            .iter()
            .map(String::as_str)
            .filter_map(event_from_line)
            .collect::<Vec<_>>(),
    );
}

mod a2a_stream_tests_behavior;
