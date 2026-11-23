use std::fs::read;
use std::path::PathBuf;

use rstest::rstest;

use crate::model_usage::decode::decode_model_usage_fields;
use crate::request_usage::baseline_capture::decode_captured_baseline;
use crate::window_queue::stamp::decode_window_stamp;

const VALID: &[u8] =
    include_bytes!("../../../../fuzz/corpus/rollup_inputs/captured-baseline-valid");
const IDENTITY: &[u8] =
    include_bytes!("../../../../fuzz/corpus/rollup_inputs/captured-baseline-identity");
const EXTRA_FIELD: &[u8] =
    include_bytes!("../../../../fuzz/corpus/rollup_inputs/captured-baseline-extra-field");
const OBSERVATION_EXTRA_FIELD: &[u8] = include_bytes!(
    "../../../../fuzz/corpus/rollup_inputs/captured-baseline-observation-extra-field"
);
const MISSING_SESSION: &[u8] =
    include_bytes!("../../../../fuzz/corpus/rollup_inputs/captured-baseline-missing-session");
const EMPTY_REQUEST_ID: &[u8] =
    include_bytes!("../../../../fuzz/corpus/rollup_inputs/captured-baseline-empty-request-id");
const PRICES_NOT_A_MAP: &[u8] =
    include_bytes!("../../../../fuzz/corpus/rollup_inputs/captured-baseline-prices-not-a-map");

#[rstest]
fn replays_the_rollup_inputs_corpus(#[files("fuzz/corpus/rollup_inputs/*")] path: PathBuf) {
    replay_rollup_input(&read(path).expect("a corpus file"));
}

// The tagged dispatch of fuzz/fuzz_targets/rollup_inputs.rs, captured baseline cases included.
fn replay_rollup_input(bytes: &[u8]) {
    let Some((&tag, rest)) = bytes.split_first() else {
        return;
    };
    if (tag & 2) != 0 {
        drop(decode_captured_baseline(rest));
        return;
    }
    if (tag & 1) == 0 {
        drop(decode_window_stamp(rest));
    } else {
        let mut parts = rest.split(|byte| *byte == 0);
        let metadata = parts.next().unwrap_or_default();
        let request_tags = parts.next().unwrap_or_default();
        drop(decode_model_usage_fields(metadata, request_tags));
    }
}

// The tag the captured baseline cases carry, with the JSON the decoder owns.
fn captured_json(bytes: &'static [u8]) -> &'static [u8] {
    let (tag, rest) = bytes.split_first().expect("a tagged case");
    assert_eq!(*tag, 2);
    rest
}

#[test]
fn the_captured_baseline_corpus_decodes_the_cases_that_are_valid() {
    let valid = decode_captured_baseline(captured_json(VALID)).unwrap();
    assert_eq!(valid.api_key, "sk-capture");
    assert_eq!(valid.session_id, "sess-1");
    assert_eq!(valid.observation.request_id, "req-capture");
    let identity = decode_captured_baseline(captured_json(IDENTITY)).unwrap();
    assert_eq!(identity.api_key, "sk-other");
    assert_eq!(identity.session_id, "sess-2");
}

#[test]
fn the_captured_baseline_corpus_rejects_each_malformed_case() {
    for bytes in [
        EXTRA_FIELD,
        OBSERVATION_EXTRA_FIELD,
        MISSING_SESSION,
        EMPTY_REQUEST_ID,
        PRICES_NOT_A_MAP,
    ] {
        assert!(decode_captured_baseline(captured_json(bytes)).is_err());
    }
}
