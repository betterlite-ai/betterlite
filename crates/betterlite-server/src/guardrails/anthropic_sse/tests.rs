#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_guardrails::guardrail_message_scope::StreamItem;
use rstest::rstest;

use super::is_anthropic_sse_stream::is_anthropic_sse_stream;
use super::is_raw_sse_stream::is_raw_sse_stream;
use super::is_sse_error_stream::is_sse_error_stream;
use super::sse_stream::{anthropic_message_start, joined_sse_stream, parsed_sse_events};

pub(crate) mod support;

fn corpus_chunks(file: &Path) -> [StreamItem; 1] {
    [StreamItem::Bytes(fs::read(file).unwrap())]
}

#[rstest]
fn replays_the_anthropic_sse_corpus(#[files("fuzz/corpus/anthropic_sse/*")] path: PathBuf) {
    let chunks = corpus_chunks(&path);
    assert!(is_raw_sse_stream(&chunks));
    let _anthropic = is_anthropic_sse_stream(&chunks);
    let _errors = is_sse_error_stream(&chunks);
    if let Some(stream) = joined_sse_stream(&chunks) {
        let events = parsed_sse_events(&stream);
        let _start = anthropic_message_start(&events);
    }
}

#[rstest]
#[case::message_start_and_ping("message_start_and_ping.txt", true, false)]
#[case::anthropic_error("anthropic_error.txt", true, true)]
#[case::chat_error("chat_error.txt", false, true)]
#[case::google_and_malformed("google_and_malformed.txt", false, false)]
#[case::invalid_utf8("invalid_utf8.bin", false, false)]
#[case::cut_off_character("cut_off_character.bin", false, false)]
fn the_corpus_files_classify_as_expected(
    #[case] file: &str,
    #[case] anthropic: bool,
    #[case] errors_only: bool,
) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/anthropic_sse")
        .join(file);
    let chunks = corpus_chunks(&path);
    assert_eq!(is_anthropic_sse_stream(&chunks), anthropic);
    assert_eq!(is_sse_error_stream(&chunks), errors_only);
}
