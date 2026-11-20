#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;
use serde_json::json;

use super::Op;

#[rstest]
fn matches_the_pinned_request_body_safety_policy(
    #[base_dir = "../../.."]
    #[files("cases/helpers/request-body-safety/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).expect("case file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("case runs"),
        HelperRun::Match
    ));
}

#[test]
fn helper_seeds_select_request_body_safety() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("request-body-safety"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "request_body_safety"
            && Op::check(&case.call.input).is_ok()
    }));
}

#[test]
fn a_banned_field_is_reported_as_value_error() {
    let error = Op::decode_and_run(&json!({
        "request_body": {"model": "gpt-4o", "api_base": "https://evil.example"},
        "general_settings": {},
        "model": "gpt-4o",
        "model_list": [],
    }))
    .expect_err("the production policy refuses api_base");
    assert_eq!(error.class, "ValueError");
    assert_eq!(error.status, None);
    assert!(
        error
            .message
            .contains("api_base is not allowed in request body")
    );
    assert!(!error.message.contains("evil.example"));
}

#[test]
fn an_object_without_banned_fields_is_safe() {
    assert_eq!(
        Op::decode_and_run(&json!({
            "request_body": {"model": "gpt-4o", "messages": []},
            "general_settings": {"allow_client_side_credentials": true},
            "model": "gpt-4o",
            "model_list": [],
        }))
        .expect("the body is safe"),
        json!(true)
    );
}
