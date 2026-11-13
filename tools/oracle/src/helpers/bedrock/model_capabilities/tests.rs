#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::Helper as _;

use super::Op;
use super::input::Input;

const OPERATION: &str = "bedrock_model_capabilities";
const SEED_PREFIX: &str = "bedrock_model_capabilities";

#[test]
fn helper_seeds_select_bedrock_model_capabilities() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut seeds = Vec::new();
    for entry in fs::read_dir(corpus).expect("the corpus folder is readable") {
        let path = entry.expect("a corpus entry").path();
        let prefixed = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(SEED_PREFIX));
        if !prefixed {
            continue;
        }
        let source = fs::read_to_string(&path).expect("a seed is readable");
        seeds.push((path, source));
    }
    assert!(!seeds.is_empty(), "no {SEED_PREFIX} seed exists");
    for (path, source) in &seeds {
        let case: HelperCase = serde_saphyr::from_str(source).expect("a seed parses");
        validate_helper(path, &case).expect("a seed validates");
        assert_eq!(case.call.operation, OPERATION);
        let _: Input =
            serde_json::from_value(case.call.input.clone()).expect("a seed names a capability");
    }
}

#[test]
fn an_unknown_function_is_not_a_bedrock_model_capabilities_input() {
    let input = serde_json::json!({"function": "nope"});
    assert!(Op::check(&input).is_err());
}

#[test]
fn a_capability_name_selects_its_variant() {
    let input = serde_json::json!({
        "function": "native_responses",
        "model_cost": {"listed": {"supported_endpoints": ["/v1/responses"]}},
        "models": ["listed", null],
    });
    Op::check(&input).expect("a well-typed input should check");
}
