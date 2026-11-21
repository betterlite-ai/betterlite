#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::ORACLE_DIR;
use serde_json::json;

use super::{Input, Op};

#[test]
fn helper_seeds_select_foundry_rejected_fields() {
    let corpus = Path::new(ORACLE_DIR).join("fuzz/corpus/helper_case");
    let seeds: Vec<_> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("foundry_rejected_fields"))
        })
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        serde_saphyr::from_str(source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| {
                validate_helper(Path::new("<helper>"), &parsed).map_err(|error| error.to_string())
            })
            .is_ok()
            && serde_saphyr::from_str::<HelperCase>(source)
                .unwrap()
                .call
                .operation
                == "foundry_rejected_fields"
    }));
}

#[test]
fn the_helper_reports_every_named_section() {
    let input: Input = serde_json::from_str(
        r#"{
            "triggers": [
                {"error": "Extra inputs are not permitted", "request_drop_params": false, "global_drop_params": false},
                {"error": "unknown field: parameter index is not a valid field", "request_drop_params": false, "global_drop_params": false}
            ],
            "repairs": [
                {"error": "{\"detail\":[{\"loc\":[\"body\",\"temperature\"]}]}", "body": {"model": "deployment", "temperature": 2}}
            ],
            "bounds": true
        }"#,
    )
    .unwrap();
    let answer = Op::run(&input).unwrap();
    assert_eq!(
        answer.get("triggers").unwrap(),
        &json!([{"retry": false}, {"retry": true}])
    );
    assert_eq!(
        answer.get("repairs").unwrap(),
        &json!([{"body": {"model": "deployment"}}])
    );
    assert_eq!(answer.get("bounds").unwrap(), &json!({"max_retry": 2}));
}

#[test]
fn a_section_the_input_does_not_name_is_left_out() {
    let input: Input = serde_json::from_str(r#"{"bounds": false}"#).unwrap();
    let answer = Op::run(&input).unwrap();
    assert_eq!(answer, json!({}));
}

#[test]
fn a_repair_that_cannot_be_read_reports_the_error() {
    let input: Input =
        serde_json::from_str(r#"{"repairs": [{"error": "not json", "body": {}}]}"#).unwrap();
    let answer = Op::run(&input).unwrap();
    let reports = answer.get("repairs").unwrap().as_array().unwrap();
    let message = reports
        .first()
        .unwrap()
        .get("error")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(message.starts_with("the error response is not JSON: "));
}
