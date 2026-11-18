#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Input;

#[test]
fn replays_the_responses_stream_hooks_seeds() {
    let source = fs::read_to_string(
        Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case/responses_stream_hooks-cache.yaml"),
    )
    .unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    assert_eq!(case.call.operation, "responses_stream_hooks");
    validate_helper(Path::new("<helper>"), &case).unwrap();
    serde_json::from_value::<Input>(case.call.input).unwrap();
}
