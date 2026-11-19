#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;

use super::Op;
use super::input::VARIANTS;

#[test]
fn helper_seeds_select_bedrock_mantle_settings() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bedrock_mantle_settings"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        validate_helper(Path::new("<helper>"), &case).unwrap();
        assert_eq!(case.call.operation, "bedrock_mantle_settings");
        let named = VARIANTS
            .iter()
            .filter(|variant| case.call.input.get(*variant).is_some())
            .count();
        assert_eq!(named, 1, "a seed names exactly one operation variant");
        <Op as Helper>::check(&case.call.input).unwrap();
        true
    }));
}
