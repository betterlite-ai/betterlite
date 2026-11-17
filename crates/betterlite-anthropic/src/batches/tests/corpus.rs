use std::fs;
use std::path::Path;

use crate::batches::decode::{BatchBody, DecodeError, DecodedBody, decode_anthropic_batch_result};

#[derive(Clone, Copy)]
enum Expected {
    Lifecycle,
    Result,
    Parse,
    Overflow,
    Shape,
}

fn expected_of(name: &str) -> Expected {
    match name {
        "valid-lifecycle.json" => Expected::Lifecycle,
        "valid-result.jsonl" => Expected::Result,
        "malformed-lifecycle.json" | "truncated-lifecycle.json" => Expected::Parse,
        "boundary-counts.json" => Expected::Overflow,
        "structural-result.jsonl" => Expected::Shape,
        other => panic!("unexpected anthropic_batches corpus seed {other}"),
    }
}

// the fuzz target reads the same first byte: R selects the result rows, anything else the lifecycle body
fn tagged(bytes: &[u8]) -> (BatchBody, &[u8]) {
    match bytes.split_first() {
        Some((b'R', body)) => (BatchBody::Result, body),
        Some((_, body)) => (BatchBody::Lifecycle, body),
        None => (BatchBody::Lifecycle, bytes),
    }
}

#[test]
fn replays_the_anthropic_batches_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/anthropic_batches");
    let sources: Vec<(String, Vec<u8>)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .to_owned();
            (name, fs::read(path).unwrap())
        })
        .collect();
    assert!(sources.len() >= 4, "{} seeds", sources.len());
    let mut successes = 0;
    for (name, bytes) in &sources {
        let (operation, body) = tagged(bytes);
        let decoded = decode_anthropic_batch_result(operation, body);
        successes += replay(name, decoded);
    }
    assert!(successes >= 2, "both valid seeds decode: {successes}");
}

fn replay(name: &str, decoded: Result<DecodedBody, DecodeError>) -> usize {
    match (expected_of(name), decoded) {
        (Expected::Lifecycle, Ok(DecodedBody::Lifecycle(batch))) => {
            assert_eq!(batch.id, "msgbatch_corpus");
            assert_eq!(batch.counts.succeeded, 2);
            1
        }
        (Expected::Result, Ok(DecodedBody::Results(rows))) => {
            assert_eq!(rows.len(), 2);
            1
        }
        (Expected::Parse, Err(DecodeError::Parse { .. }))
        | (Expected::Overflow, Err(DecodeError::CountOverflow))
        | (Expected::Shape, Err(DecodeError::Shape { .. })) => 0,
        (_, decoded) => panic!("{name}: unexpected decode {decoded:?}"),
    }
}
