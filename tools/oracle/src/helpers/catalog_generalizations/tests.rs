#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, parse_helper_case};
use crate::helpers::HelperCall;

#[test]
fn helper_seeds_select_catalog_generalizations() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<HelperCase> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_str().unwrap().contains("/catalog_generalizations"))
        .map(|path| {
            let source = fs::read_to_string(path).unwrap();
            parse_helper_case(&source).unwrap();
            serde_saphyr::from_str(&source).unwrap()
        })
        .collect();
    let mut outputs: Vec<String> = seeds
        .iter()
        .filter_map(|seed| {
            let HelperCall::CatalogGeneralizations(input) = &seed.call else {
                return None;
            };
            serde_json::to_string(&super::invoke(input)).ok()
        })
        .collect();
    outputs.sort();
    assert_eq!(
        outputs,
        [
            r#"{"ok":true,"value":{"capabilities":[],"fill_missing":[],"routing":[null],"rules":[],"warnings":[]}}"#,
            r#"{"ok":true,"value":{"capabilities":[{"litellm_provider":"anthropic","mode":"chat","supports_vision":true}],"fill_missing":[{"mode":"chat","supports_vision":true}],"routing":["anthropic",null],"rules":[{"model_info":{"litellm_provider":"anthropic","mode":"chat"},"name":"base","pattern":"^claude-"},{"extends":"base","fill_missing_for_providers":["anthropic"],"model_info":{"supports_vision":true},"name":"child","pattern":"claude-x"},{"model_info":{},"pattern":"("},7],"warnings":[{"message":"LiteLLM: skipping fallback generalization rule with invalid regex '(': Parsing error at position 1: Opening parenthesis without closing parenthesis","subject":"("}]}}"#,
        ]
    );
}
