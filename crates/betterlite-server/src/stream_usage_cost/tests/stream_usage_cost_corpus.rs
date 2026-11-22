use std::fs;
use std::path::PathBuf;

use std::str::from_utf8;

use rstest::rstest;
use rust_decimal::Decimal;

use super::super::StreamCostInjector;
use super::super::decode_cost_frame;
use super::super::testing::{FixedCalculator, FixedRecorder};

#[rstest]
fn replays_the_stream_usage_cost_corpus(#[files("fuzz/corpus/stream_usage_cost/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let recorder = FixedRecorder(Some(Decimal::ZERO));
    let calculator = FixedCalculator(Some(Decimal::ZERO));
    let injector = StreamCostInjector {
        enabled: true,
        recorder: &recorder,
        calculator: &calculator,
    };
    let decoded = decode_cost_frame(injector, &bytes, "gpt-4o", None);
    if let Ok(source) = from_utf8(&bytes) {
        let output = from_utf8(&decoded).unwrap();
        assert_eq!(source.matches('\n').count(), output.matches('\n').count());
    }
}

#[test]
fn injects_a_cost_into_a_complete_corpus_frame() {
    let bytes = fs::read("fuzz/corpus/stream_usage_cost/lf-frames").unwrap();
    let recorder = FixedRecorder(Some(Decimal::ZERO));
    let calculator = FixedCalculator(Some(Decimal::ZERO));
    let injector = StreamCostInjector {
        enabled: true,
        recorder: &recorder,
        calculator: &calculator,
    };
    let decoded = decode_cost_frame(injector, &bytes, "gpt-4o", None);
    let text = from_utf8(&decoded).unwrap();
    assert!(text.contains("\"cost\":0.0"));
}
