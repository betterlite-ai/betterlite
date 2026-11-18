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
    let mut rejected: Vec<String> = Vec::new();
    let mut functions: BTreeSet<String> = BTreeSet::new();
    for source in &sources {
        let case = seed_case(source);
        for call in iter::once(&case.call).chain(&case.calls) {
            if let Err(error) = <Op as Helper>::check(&call.input) {
                rejected.push(error);
            }
            if let Some(name) = call
                .input
                .get("function")
                .and_then(serde_json::Value::as_str)
            {
                functions.insert(name.to_owned());
            }
        }
    }
    assert!(rejected.is_empty(), "{rejected:#?}");
    assert_eq!(functions, BTreeSet::from(FUNCTIONS.map(String::from)));
}
