#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use rstest::rstest;

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};

#[rstest]
fn helper_seeds_select_deepinfra_policy(
    #[base_dir = "../../.."]
    #[files("fuzz/corpus/helper_case/deepinfra_policy*")]
    path: PathBuf,
) {
    let source = fs::read_to_string(&path).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "deepinfra_policy");
}

#[test]
fn helper_seeds_cover_chat_and_rerank_operations() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let chat_source = fs::read_to_string(corpus.join("deepinfra_policy-chat-map.yaml")).unwrap();
    let chat: HelperCase = serde_saphyr::from_str(&chat_source).unwrap();
    assert_eq!(
        chat.call
            .input
            .get("operation")
            .and_then(serde_json::Value::as_str),
        Some("chat_map")
    );
    let rerank_source = fs::read_to_string(corpus.join("deepinfra_policy-rerank.yaml")).unwrap();
    let rerank: HelperCase = serde_saphyr::from_str(&rerank_source).unwrap();
    assert!(
        rerank
            .call
            .input
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|operation| operation.starts_with("rerank"))
    );
}

#[rstest]
fn matches_pinned_deepinfra_policy(
    #[base_dir = "../../.."]
    #[files("cases/helpers/deepinfra-policy/*.yaml")]
    path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).unwrap(),
        HelperRun::Match
    ));
}
