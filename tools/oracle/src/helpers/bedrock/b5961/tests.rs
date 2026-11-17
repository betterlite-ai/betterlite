#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::iter;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;

// every branch needs a seed, so a new function cannot land without one
const FUNCTIONS: [&str; 8] = [
    "model_config",
    "headers",
    "hf_request",
    "hf_reply",
    "cohere_map",
    "cohere_request",
    "cohere_reply",
    "decode_stream",
];

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("b5961"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect()
}

fn seed_case(source: &str) -> HelperCase {
    let case = serde_saphyr::from_str::<HelperCase>(source).unwrap();
    validate_helper(Path::new("<helper>"), &case).unwrap();
    assert_eq!(case.call.operation, "b5961");
    case
}

#[test]
fn seeds_select_b5961() {
    let sources = seed_sources();
    assert!(!sources.is_empty());
    let cases: Vec<HelperCase> = sources.iter().map(|source| seed_case(source)).collect();
    let inputs: Vec<&serde_json::Value> = cases
        .iter()
        .flat_map(|case| iter::once(&case.call).chain(&case.calls))
        .map(|call| &call.input)
        .collect();
    let rejected: Vec<String> = inputs
        .iter()
        .copied()
        .filter_map(|input| <Op as Helper>::check(input).err())
        .collect();
    assert!(rejected.is_empty(), "{rejected:#?}");
    let functions: BTreeSet<&str> = inputs
        .iter()
        .filter_map(|input| input.get("function").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(functions, BTreeSet::from(FUNCTIONS));
}
