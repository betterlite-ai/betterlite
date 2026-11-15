#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::Helper as _;

use super::Op;
use super::input::Input;

const OPERATION: &str = "bedrock_converse_options";
const SEED_PREFIX: &str = "bedrock_converse_options";

#[test]
fn helper_seeds_select_bedrock_converse_options() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .expect("the corpus folder is readable")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(SEED_PREFIX))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).expect("a seed is readable");
            (path, source)
        })
        .collect();
    assert!(!seeds.is_empty(), "no {SEED_PREFIX} seed exists");
    assert!(
        seeds.iter().all(|(path, source)| {
            let case: HelperCase = serde_saphyr::from_str(source).expect("a seed parses");
            validate_helper(path, &case).expect("a seed validates");
            assert_eq!(case.call.operation, OPERATION);
            serde_json::from_value::<Input>(case.call.input)
                .map(drop)
                .is_ok()
        }),
        "a seed names a converse option function"
    );
}

#[test]
fn an_unknown_function_is_not_a_bedrock_converse_options_input() {
    let input = serde_json::json!({"function": "nope"});
    assert!(Op::check(&input).is_err());
}

#[test]
fn a_function_name_selects_its_variant() {
    let input = serde_json::json!({
        "function": "flags",
        "kind": "nova2",
        "model_cost": {},
        "models": ["amazon.nova-2-lite-v1:0"],
    });
    Op::check(&input).expect("a well-typed input should check");
}
