use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;

use crate::batches::decode::{BatchBody, DecodeError, DecodedBody, decode_anthropic_batch_result};

#[derive(Clone, Copy)]
enum Expected {
    Lifecycle,
    Result,
    Parse,
    Overflow,
    Shape,
}

// the fuzz target reads the same first byte: R selects the result rows, anything else the lifecycle body
fn tagged(bytes: &[u8]) -> (BatchBody, &[u8]) {
    match bytes.split_first() {
        Some((b'R', body)) => (BatchBody::Result, body),
        Some((_, body)) => (BatchBody::Lifecycle, body),
        None => (BatchBody::Lifecycle, bytes),
    }
}

fn seed(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fuzz/corpus/anthropic_batches")
        .join(name)
}

#[rstest]
#[case::valid_lifecycle("valid-lifecycle.json", Expected::Lifecycle)]
#[case::valid_result("valid-result.jsonl", Expected::Result)]
#[case::malformed_lifecycle("malformed-lifecycle.json", Expected::Parse)]
#[case::truncated_lifecycle("truncated-lifecycle.json", Expected::Parse)]
#[case::boundary_counts("boundary-counts.json", Expected::Overflow)]
#[case::structural_result("structural-result.jsonl", Expected::Shape)]
fn replays_the_anthropic_batches_corpus(#[case] name: &str, #[case] expected: Expected) {
    let bytes = fs::read(seed(name)).unwrap();
    let (operation, body) = tagged(&bytes);
    match (expected, decode_anthropic_batch_result(operation, body)) {
        (Expected::Lifecycle, Ok(DecodedBody::Lifecycle(batch))) => {
            assert_eq!(batch.id, "msgbatch_corpus");
            assert_eq!(batch.counts.succeeded, 2);
        }
        (Expected::Result, Ok(DecodedBody::Results(rows))) => assert_eq!(rows.len(), 2),
        (Expected::Parse, Err(DecodeError::Parse { .. }))
        | (Expected::Overflow, Err(DecodeError::CountOverflow))
        | (Expected::Shape, Err(DecodeError::Shape { .. })) => {}
        (_, decoded) => panic!("{name}: unexpected decode {decoded:?}"),
    }
}
