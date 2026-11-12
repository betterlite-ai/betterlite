#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;

use super::Op;

#[test]
fn helper_seeds_select_bedrock_services_batch_status() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .expect("the helper corpus exists")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_services_batch_status"))
        })
        .map(|path| fs::read_to_string(path).expect("the seed reads"))
        .collect();
    assert!(
        !seeds.is_empty(),
        "no bedrock_services_batch_status seed exists"
    );
    assert!(seeds.iter().all(|source| {
        let parsed = serde_saphyr::from_str::<HelperCase>(source).expect("the seed parses");
        validate_helper(Path::new("<helper>"), &parsed).is_ok()
            && parsed.call.operation == "bedrock_services_batch_status"
            && Op::check(&parsed.call.input).is_ok()
    }));
}

#[test]
fn an_unknown_operation_is_not_an_input() {
    let input = serde_json::json!({"operation": "nope"});
    assert!(Op::check(&input).is_err());
}
