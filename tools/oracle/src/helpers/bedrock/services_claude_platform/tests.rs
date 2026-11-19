#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::input::Input;

const OPERATION: &str = "bedrock_services_claude_platform";

#[test]
fn helper_seeds_select_bedrock_services_claude_platform() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, HelperCase)> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_services_claude_platform"))
        })
        .map(|path| {
            let source = fs::read_to_string(&path).unwrap();
            let parsed = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
            (path.display().to_string(), parsed)
        })
        .collect();
    assert!(!seeds.is_empty(), "no claude platform helper seeds");
    assert!(seeds.iter().all(|(path, seed)| {
        validate_helper(Path::new(path), seed)
            .map_err(|error| error.to_string())
            .is_ok()
            && seed.call.operation == OPERATION
            && serde_json::from_value::<Input>(seed.call.input.clone()).is_ok()
    }));
}
