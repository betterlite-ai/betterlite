#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::iter;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;

fn seed_case(name: &str) -> HelperCase {
    let path = Path::new(ORACLE_DIR)
        .join("fuzz/corpus/helper_case")
        .join(name);
    let source = fs::read_to_string(path).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    validate_helper(Path::new("<helper>"), &case).unwrap();
    assert_eq!(case.call.operation, "b5961");
    case
}

// each branch has its own seed, so a new function cannot land without a case here
#[rstest::rstest]
#[case::model_config("b5961-model-config.yaml", "model_config")]
#[case::headers("b5961-headers.yaml", "headers")]
#[case::hf_request("b5961-hf-request.yaml", "hf_request")]
#[case::hf_reply("b5961-hf-reply.yaml", "hf_reply")]
#[case::cohere_map("b5961-cohere-map.yaml", "cohere_map")]
#[case::cohere_request("b5961-cohere-request.yaml", "cohere_request")]
#[case::cohere_reply("b5961-cohere-reply.yaml", "cohere_reply")]
#[case::decode_stream("b5961-decode-stream.yaml", "decode_stream")]
fn seeds_select_b5961(#[case] name: &str, #[case] function: &str) {
    let case = seed_case(name);
    let rejected: Vec<String> = iter::once(&case.call)
        .chain(&case.calls)
        .filter_map(|call| <Op as Helper>::check(&call.input).err())
        .collect();
    assert!(rejected.is_empty(), "{name}: {rejected:#?}");
    let functions: BTreeSet<&str> = iter::once(&case.call)
        .chain(&case.calls)
        .filter_map(|call| {
            call.input
                .get("function")
                .and_then(serde_json::Value::as_str)
        })
        .collect();
    assert_eq!(functions, BTreeSet::from([function]), "{name}");
}
