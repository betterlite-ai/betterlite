#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_azure_responses() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("azure_responses"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        let variant = case
            .call
            .input
            .get("function")
            .and_then(serde_json::Value::as_str)
            .unwrap();
        case.call.operation == "azure_responses"
            && [
                "base_and_api_version",
                "strip_model_prefix",
                "o_series_options",
                "reasoning_null_fields",
                "websocket_model_query",
                "resource_id_before_query",
                "cancel_response",
                "azure_responses_capabilities",
            ]
            .contains(&variant)
            && validate_helper(Path::new("<helper>"), &case).is_ok()
    }));
}
