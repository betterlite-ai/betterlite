#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;
use super::input::Input;

const OPERATION: &str = "bedrock_converse_messages";
const SEED_PREFIX: &str = "bedrock_converse_messages";

#[test]
fn helper_seeds_select_bedrock_converse_messages() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
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
        "a seed names a converse message policy function"
    );
}

#[test]
fn an_unknown_function_is_not_a_bedrock_converse_messages_input() {
    let input = serde_json::json!({"function": "nope"});
    assert!(Op::check(&input).is_err());
}

#[test]
fn a_function_name_selects_its_variant() {
    let guarded: Input =
        serde_json::from_value(serde_json::json!({"function": "guarded", "messages": []}))
            .expect("the guarded input parses");
    assert!(matches!(guarded, Input::Guarded(_)));
    let handled: Input = serde_json::from_value(serde_json::json!({
        "function": "handle_orphaned",
        "messages": [],
        "neutralize_orphaned_blocks": true,
        "modify_params": false
    }))
    .expect("the orphan input parses");
    assert!(matches!(handled, Input::HandleOrphaned(_)));
    let system: Input =
        serde_json::from_value(serde_json::json!({"function": "transform_system", "messages": []}))
            .expect("the system input parses");
    assert!(matches!(system, Input::TransformSystem(_)));
}
