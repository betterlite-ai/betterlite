use super::*;

use axum::http::StatusCode;

use crate::model_listing::ModelListingError;
use crate::model_listing::info::validate_model_access;

#[test]
fn a_listed_single_model_is_accessible() {
    validate_model_access("a", &strings(&["a", "b"])).expect("the model is accessible");
}

#[test]
fn an_unlisted_single_model_is_refused() {
    let error = validate_model_access("b", &strings(&["a"])).expect_err("the model is refused");
    assert_eq!(
        error.to_string(),
        "The model `b` does not exist or is not accessible"
    );
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[test]
fn a_batch_of_listed_models_is_accessible() {
    validate_model_access("a, b", &strings(&["a", "b"])).expect("the models are accessible");
}

#[test]
fn a_batch_names_every_unlisted_model() {
    let error = validate_model_access("a, c, d", &strings(&["a", "b"]))
        .expect_err("the models are refused");
    assert_eq!(
        error.to_string(),
        "The following model(s) do not exist or are not accessible: c, d"
    );
}

#[test]
fn a_batch_with_an_empty_element_refuses_the_whole_batch() {
    let error = validate_model_access("a, ", &strings(&["a"])).expect_err("the batch is refused");
    assert_eq!(
        error.to_string(),
        "The following model(s) do not exist or are not accessible: "
    );
}

#[test]
fn a_batch_trims_the_names_it_checks() {
    let error =
        validate_model_access(" a , c ", &strings(&["a"])).expect_err("the models are refused");
    assert_eq!(
        error.to_string(),
        "The following model(s) do not exist or are not accessible: c"
    );
}

fn assert_error_statuses(cases: Vec<(ModelListingError, StatusCode)>) {
    for (error, status) in cases {
        assert_eq!(error.status(), status, "{error}");
    }
}

#[test]
fn the_listing_error_statuses_follow_the_lite_llm_exceptions() {
    let key_fields = serde_json::from_str::<u8>("bad").expect_err("the fixture is not a number");
    let missing = TeamMissing {
        team_id: "t1".to_owned(),
    };
    let not_member = NotAMember {
        team_id: "t1".to_owned(),
    };
    assert_error_statuses(vec![
        (
            ModelListingError::TeamNotFound(missing),
            StatusCode::NOT_FOUND,
        ),
        (
            ModelListingError::NotAMember(not_member),
            StatusCode::FORBIDDEN,
        ),
        (
            ModelListingError::BatchInaccessible {
                models: strings(&["a"]),
            },
            StatusCode::NOT_FOUND,
        ),
        (
            ModelListingError::ModelInaccessible {
                model_id: "a".to_owned(),
            },
            StatusCode::NOT_FOUND,
        ),
        (
            ModelListingError::InvalidFallbackType,
            StatusCode::BAD_REQUEST,
        ),
        (
            ModelListingError::KeyFields(key_fields),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        (
            ModelListingError::Wildcard(WildcardError::Lookup),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ]);
}
