#![cfg(test)]

use std::fs;
use std::path::Path;

use crate::case::{HelperCase, helper::validate_helper};

use super::{Input, Op};

fn seed_sources() -> Vec<String> {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("legacy_vertex_plan"))
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
fn helper_seeds_select_legacy_vertex_plan() {
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
            && case.call.operation == "legacy_vertex_plan"
    }));
}

#[test]
fn seeds_reach_native_formatter() {
    let inputs = seed_inputs();
    assert!(
        inputs
            .iter()
            .any(|input| input.project.is_none() && input.location.is_none())
    );
    assert!(
        inputs
            .iter()
            .any(|input| input.project.as_deref() == Some(""))
    );
    let keys: Vec<String> = inputs
        .iter()
        .map(|input| {
            let answer = <Op as crate::helpers::Helper>::run(input).expect("a native answer");
            let key = answer.as_str().expect("a key string");
            key.to_owned()
        })
        .collect();
    assert!(keys.iter().any(|key| key.ends_with("-None-None")));
    assert!(keys.iter().any(|key| key.ends_with("--")));
    assert!(keys.iter().any(|key| key.contains("text-bison@001-p-1-")));
}
