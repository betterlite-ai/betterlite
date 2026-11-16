#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

#[rstest::rstest]
#[case::cache("gemini_request-cache.yaml")]
#[case::maps("gemini_request-maps.yaml")]
fn seeds_decode_the_gemini_request_input(#[case] name: &str) {
    let path = Path::new(ORACLE_DIR)
        .join("fuzz/corpus/helper_case")
        .join(name);
    let source = fs::read_to_string(path).expect("a seed is readable");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("a seed parses as a helper case");
    assert!(
        validate_helper(Path::new("<helper>"), &case).is_ok(),
        "{name} is a valid helper case"
    );
    assert_eq!(
        case.call.operation, "gemini_request",
        "{name} names the right operation"
    );
    let input: super::ops::Input =
        serde_json::from_value(case.call.input).expect("the seed input has the shape");
    assert!(!input.model.is_empty(), "{name} names a model");
    assert!(!input.messages.is_empty(), "{name} carries messages");
}
