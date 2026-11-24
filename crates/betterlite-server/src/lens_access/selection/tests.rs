#![cfg(test)]

use axum::http::StatusCode;

use crate::lens_access::error::failure_text;
use crate::lens_access::selection::validate_selection;
use crate::lens_access::tests::support::{execution_id, settings, settings_with_ids};

const INVALID: &str = "Choose execution IDs returned by the activity preview";

#[test]
fn no_execution_ids_pass() {
    validate_selection(&settings("gpt-4o")).expect("an empty selection passes");
}

#[test]
fn traces_and_requests_ids_pass() {
    let trace = execution_id(&["traces", "team-1", "trace-1"]);
    let request = execution_id(&["requests", "team-1", "trace-1", "ref"]);
    let chosen = settings_with_ids(&[trace.as_str(), request.as_str()]);
    validate_selection(&chosen).expect("both sources pass");
}

#[test]
fn a_broken_id_is_rejected() {
    let chosen = settings_with_ids(&["not base64 at all"]);
    let error = validate_selection(&chosen).expect_err("the id is broken");
    assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(failure_text(&error), INVALID);
}

#[test]
fn an_unknown_source_is_rejected() {
    let logs = execution_id(&["logs", "team-1", "trace-1"]);
    let chosen = settings_with_ids(&[logs.as_str()]);
    let error = validate_selection(&chosen).expect_err("the source is unknown");
    assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(failure_text(&error), INVALID);
}

#[test]
fn an_id_with_the_wrong_element_count_is_rejected() {
    let short = execution_id(&["traces", "team-1"]);
    let chosen = settings_with_ids(&[short.as_str()]);
    let error = validate_selection(&chosen).expect_err("the id is short");
    assert_eq!(error.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
