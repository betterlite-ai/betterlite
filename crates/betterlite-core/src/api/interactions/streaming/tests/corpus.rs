use std::collections::VecDeque;
use std::fs;
use std::mem;
use std::path::Path;

use betterlite_core::api::interactions::streaming::base::{ParsedStreamChunk, parse_stream_chunk};
use betterlite_openai::aleph_alpha::lines::LineDecoder;

const CORPUS_SEEDS: usize = 10;

#[test]
fn replays_the_interactions_sse_corpus() {
    let seeds = corpus_seeds();
    assert_eq!(seeds.len(), CORPUS_SEEDS);
    replay_all(seeds);
}

fn replay_all(seeds: Vec<(String, Vec<u8>)>) {
    for (name, bytes) in seeds {
        let whole = replay(vec![bytes.clone()]);
        let byte_wise: Vec<Vec<u8>> = bytes.chunks(1).map(<[u8]>::to_vec).collect();
        assert_eq!(
            replay(byte_wise),
            whole,
            "{name} parses the same byte by byte"
        );
        assert_eq!(
            buffered(&bytes),
            whole,
            "{name} parses the same through the buffered response"
        );
        check_corpus_expectation(&name, &whole);
    }
}

fn corpus_seeds() -> Vec<(String, Vec<u8>)> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/interactions_sse");
    let mut seeds: Vec<(String, Vec<u8>)> = Vec::new();
    for dir_entry in fs::read_dir(corpus).expect("the corpus folder") {
        let entry = dir_entry.expect("a corpus entry");
        let name = entry
            .file_name()
            .to_str()
            .expect("the seed name is UTF-8")
            .to_owned();
        if name.starts_with('.') {
            continue;
        }
        let bytes = fs::read(entry.path()).expect("a seed file");
        seeds.push((name, bytes));
    }
    seeds.sort();
    seeds
}

fn replay(chunks: Vec<Vec<u8>>) -> Vec<ParsedStreamChunk> {
    let mut decoder = LineDecoder::new(MAX_LINE_BYTES);
    let mut pending = VecDeque::new();
    let mut lines: Vec<String> = Vec::new();
    for chunk in chunks {
        decoder
            .push(&chunk, &mut pending)
            .expect("the corpus seed stays under the line cap");
        lines.extend(mem::take(&mut pending));
    }
    decoder.flush(&mut pending);
    lines.extend(mem::take(&mut pending));
    lines
        .iter()
        .map(|line| parse_stream_chunk(line.as_str()))
        .collect()
}

fn buffered(bytes: &[u8]) -> Vec<ParsedStreamChunk> {
    let response = InteractionBufferedResponse {
        headers: IndexMap::new(),
        body: bytes.to_vec(),
    };
    response
        .iter_lines()
        .map(|line| parse_stream_chunk(line.expect("the corpus seed decodes").as_str()))
        .collect()
}

fn check_corpus_expectation(name: &str, parsed: &[ParsedStreamChunk]) {
    match name {
        "blank-lines" => assert_eq!(parsed, [ParsedStreamChunk::Skip, ParsedStreamChunk::Skip]),
        "crlf-lines" => assert_eq!(parsed, [event("a"), event("b")]),
        "done-marker" => assert_eq!(parsed, [ParsedStreamChunk::Done]),
        "empty" => assert!(parsed.is_empty()),
        "event" => assert_eq!(parsed, [event("int_1")]),
        "invalid-utf8" => assert_eq!(parsed, [event("\u{fffd}")]),
        "malformed-json" | "non-object" => assert_eq!(parsed, [ParsedStreamChunk::Skip]),
        "split-utf8" => assert_eq!(parsed, [event("\u{e9}")]),
        "unterminated-final" => {
            assert_eq!(parsed, [event("a"), ParsedStreamChunk::Skip, event("b")]);
        }
        other => panic!("the corpus seed {other} has no expectation"),
    }
}

fn event(id: &str) -> ParsedStreamChunk {
    let fields = IndexMap::from([(String::from("id"), JsonValue::Text(id.to_owned()))]);
    ParsedStreamChunk::Event(fields)
}
