#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;
use serde_json::Value;

use super::Input;

#[derive(serde::Deserialize)]
struct SeedCall {
    call: BTreeMap<String, Value>,
}

#[test]
fn helper_seeds_select_fal_render_profiles() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .expect("the helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("fal_render_profiles"))
        })
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a seed has a name")
                .to_owned();
            let source = fs::read_to_string(path).expect("a seed is readable");
            (name, source)
        })
        .collect();
    assert!(!seeds.is_empty(), "no fal_render_profiles seed exists");
    assert!(seeds.iter().all(|(name, source)| {
        let case: HelperCase =
            serde_saphyr::from_str(source).expect("a seed parses as a helper case");
        let seed: SeedCall = serde_saphyr::from_str(source).expect("the seed call parses");
        let Some(value) = seed.call.get("fal_render_profiles") else {
            return false;
        };
        let input: Input =
            serde_json::from_value(value.clone()).expect("the seed input has the typed shape");
        assert!(
            matches!(input, Input::CustomDiffusionPath { .. }),
            "{name} must carry the custom diffusion path operation"
        );
        validate_helper(Path::new("<helper>"), &case).is_ok()
    }));
}
