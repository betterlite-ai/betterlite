#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::{Function, Input};

#[test]
fn helper_seeds_select_azure_chat_call() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .expect("the helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("azure_chat_call-"))
        })
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a seed name")
                .to_owned();
            let source = fs::read_to_string(path).expect("a seed is readable");
            (name, source)
        })
        .collect();
    assert!(!seeds.is_empty(), "no azure_chat_call seed exists");
    assert!(seeds.iter().all(|(name, source)| {
        let case: HelperCase =
            serde_saphyr::from_str(source).expect("a seed parses as a helper case");
        let input: Input = serde_json::from_value(case.call.input.clone())
            .expect("the seed input has the typed shape");
        let o_series = matches!(input.function, Function::OSeries);
        o_series == name.contains("o-series")
            && case.call.operation == "azure_chat_call"
            && validate_helper(Path::new("<helper>"), &case).is_ok()
    }));
}
