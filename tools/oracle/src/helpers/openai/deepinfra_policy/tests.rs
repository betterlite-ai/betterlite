#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::{HelperRun, run_case};

#[rstest]
fn helper_seeds_select_deepinfra_policy(
    #[files("fuzz/corpus/helper_case/deepinfra_policy*")] path: PathBuf,
) {
    let source = fs::read_to_string(&path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "deepinfra_policy");
}

#[test]
fn helper_seeds_cover_chat_and_rerank_operations() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let mut operations = Vec::new();
    for entry in fs::read_dir(corpus).unwrap() {
        let path = entry.unwrap().path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("deepinfra_policy") {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
        let operation = case
            .call
            .input
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        operations.push(operation);
    }
    assert!(operations.iter().any(|operation| operation == "chat_map"));
    assert!(
        operations
            .iter()
            .any(|operation| operation.starts_with("rerank"))
    );
}

#[rstest]
fn matches_pinned_deepinfra_policy(
    #[files("cases/helpers/deepinfra-policy/*.yaml")] path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(matches!(run_case(&case, false).unwrap(), HelperRun::Match));
}
