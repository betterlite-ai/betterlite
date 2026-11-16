#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::input::Input;

#[test]
fn helper_seeds_select_bedrock_services_passthrough() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_services_passthrough"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    for source in &seeds {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        validate_helper(Path::new("<helper>"), &case).unwrap();
        assert_eq!(case.call.operation, "bedrock_services_passthrough");
        let input: Input = serde_json::from_value(case.call.input.clone()).unwrap();
        assert!(matches!(
            input,
            Input::Collector { .. } | Input::Selection { .. } | Input::Sign { .. }
        ));
    }
}
