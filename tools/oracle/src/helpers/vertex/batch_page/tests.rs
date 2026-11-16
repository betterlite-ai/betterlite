#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use serde_json::{Value, json};

use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperRun, run_case};

fn run(body: &str) -> Value {
    let input = serde_json::from_value(json!({"body": body})).unwrap();
    super::ops::run(&input).unwrap()
}

#[rstest]
fn helper_seeds_select_vertex_batch_page(
    #[base_dir = "../../.."]
    #[files("fuzz/corpus/helper_case/vertex_batch_page*")]
    path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    validate_helper(&path, &case).unwrap();
    assert_eq!(case.call.operation, "vertex_batch_page");
}

#[rstest]
fn matches_pinned_vertex_batch_page(
    #[base_dir = "../../.."]
    #[files("cases/helpers/vertex-batch-page/*.yaml")]
    path: PathBuf,
) {
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).unwrap(),
        HelperRun::Match
    ));
}

#[test]
fn a_page_body_returns_every_field_it_carries() {
    let page = run(r#"{"batchPredictionJobs":[{"name":"jobs/42"}],"nextPageToken":"cursor"}"#);
    assert_eq!(
        page,
        json!({"batchPredictionJobs": [{"name": "jobs/42"}], "nextPageToken": "cursor"})
    );
}

#[test]
fn a_job_keeps_its_state_and_create_time_and_drops_what_the_type_does_not_read() {
    assert_eq!(
        run(
            r#"{"batchPredictionJobs":[{"state":"JOB_STATE_RUNNING","createTime":"t","other":1},{"state":null}]}"#
        ),
        json!({"batchPredictionJobs": [
            {"state": "JOB_STATE_RUNNING", "createTime": "t"},
            {"state": null}
        ]})
    );
}

#[test]
fn a_null_job_list_and_an_empty_token_keep_their_keys() {
    assert_eq!(
        run(r#"{"batchPredictionJobs":null}"#),
        json!({"batchPredictionJobs": null})
    );
    assert_eq!(
        run(r#"{"batchPredictionJobs":[],"nextPageToken":""}"#),
        json!({"batchPredictionJobs": [], "nextPageToken": ""})
    );
}

#[test]
fn a_malformed_page_reports_a_value_error_without_a_status() {
    let input = serde_json::from_value(json!({"body": "{\"batchPredictionJobs\": ["})).unwrap();
    let error = super::ops::run(&input).unwrap_err();
    assert_eq!(error.class, "ValueError");
    assert_eq!(error.status, None);
}

#[test]
fn an_unknown_input_field_is_rejected() {
    let unknown =
        serde_json::from_value::<super::ops::Input>(json!({"body": "{}", "other": 1_i32}));
    assert!(unknown.unwrap_err().to_string().contains("other"));
}
