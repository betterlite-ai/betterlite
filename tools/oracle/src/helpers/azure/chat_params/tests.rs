#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

#[test]
fn helper_seeds_select_azure_chat_params() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("azure_chat_params"))
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
        case.call.operation == "azure_chat_params"
            && [
                "gpt5_alias_catalog",
                "native_stream",
                "gpt5_allowlist",
                "o_series_allowlist",
                "parameter_mapping",
                "request_transform",
            ]
            .contains(&variant)
            && validate_helper(Path::new("<helper>"), &case).is_ok()
    }));
}
