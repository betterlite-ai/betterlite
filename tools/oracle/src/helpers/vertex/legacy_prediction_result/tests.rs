#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

use super::input::{Input, Step};

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("legacy_prediction_result"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect()
}

fn seed_inputs() -> Vec<Input> {
    seed_sources()
        .iter()
        .map(|source| {
            let case: HelperCase = serde_saphyr::from_str(source).unwrap();
            serde_json::from_value(case.call.input).unwrap()
        })
        .collect()
}

#[test]
fn helper_seeds_select_legacy_prediction_result() {
    let seeds = seed_sources();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).unwrap();
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && case.call.operation == "legacy_prediction_result"
    }));
}

#[test]
fn helper_seeds_cover_both_iteration_modes() {
    let steps: Vec<Step> = seed_inputs()
        .into_iter()
        .flat_map(|input| input.steps)
        .collect();
    assert!(steps.contains(&Step::Sync));
    assert!(steps.contains(&Step::Async));
    assert!(steps.len() > 2);
}
