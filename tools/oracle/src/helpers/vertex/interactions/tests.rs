#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::case::{HelperCase, helper::validate_helper};
use crate::helpers::Helper as _;

use super::Op;

const KINDS: [&str; 3] = ["url", "lifecycle", "headers"];

fn seed_sources() -> Vec<String> {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("vertex_interactions"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect()
}

#[rstest]
fn helper_seeds_select_vertex_interactions(
    #[files("fuzz/corpus/helper_case/vertex_interactions*.yaml")] path: PathBuf,
) {
    let source = fs::read_to_string(&path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "vertex_interactions");
}

#[test]
fn helper_seeds_cover_every_vertex_interactions_call() {
    let kinds: Vec<String> = seed_sources()
        .into_iter()
        .flat_map(|source| {
            let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
            let calls = case
                .call
                .input
                .as_object()
                .unwrap()
                .get("calls")
                .unwrap()
                .as_array()
                .unwrap()
                .clone();
            calls
                .into_iter()
                .map(|call| {
                    call.as_object()
                        .unwrap()
                        .get("function")
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect::<Vec<String>>()
        })
        .collect();
    assert!(!kinds.is_empty());
    assert!(
        KINDS
            .iter()
            .all(|kind| kinds.iter().any(|found| found == kind))
    );
}

#[test]
fn a_non_interactions_input_is_refused() {
    assert!(Op::check(&serde_json::json!({"mint": 3_i32})).is_err());
}
