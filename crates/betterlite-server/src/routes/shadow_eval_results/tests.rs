#![cfg(test)]

use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use rstest::rstest;

use super::{ResultsState, answered, failure, shadow_eval_read};
use crate::shadow_eval_results::error::ShadowResultsRouteError;
use crate::shadow_eval_results::query::ShadowQueryError;
use crate::shadow_eval_validation::error::AccessError;

fn state() -> Arc<ResultsState> {
    Arc::new(ResultsState { store: None })
}

// the middleware answers an absent caller, so neither handler may run its own read first
#[tokio::test]
async fn an_unauthenticated_list_answers_401() {
    let error = shadow_eval_read(State(state()), None, None, RawQuery(None))
        .await
        .unwrap_err();
    assert_eq!(error.into_response().status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_unauthenticated_detail_answers_401() {
    let error = shadow_eval_read(
        State(state()),
        Some(Path("job-1".to_owned())),
        None,
        RawQuery(None),
    )
    .await
    .unwrap_err();
    assert_eq!(error.into_response().status(), StatusCode::UNAUTHORIZED);
}

// the read's success path reaches the caller as its JSON body, not as the empty 200 a default response carries
#[tokio::test]
async fn answered_serializes_the_read_into_the_json_body() {
    let response = answered(Ok(vec!["job-1".to_owned()]))
        .unwrap()
        .into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(String::from_utf8(body.to_vec()).unwrap(), "[\"job-1\"]");
}

// LiteLLM answers each management failure with its own status, and the body names the reason
#[rstest]
#[case::forbidden(
    ShadowResultsRouteError::Access(AccessError::ViewerForbidden {
        action: "view".to_owned()
    }),
    StatusCode::FORBIDDEN
)]
#[case::unprocessable(
    ShadowResultsRouteError::Query(ShadowQueryError::Invalid { parameter: "limit" }),
    StatusCode::UNPROCESSABLE_ENTITY
)]
#[case::filter_pair(ShadowResultsRouteError::FilterPair, StatusCode::BAD_REQUEST)]
#[case::missing(
    ShadowResultsRouteError::JobNotFound {
        job_id: "job-1".to_owned()
    },
    StatusCode::NOT_FOUND
)]
#[case::database(
    ShadowResultsRouteError::DatabaseUnavailable,
    StatusCode::INTERNAL_SERVER_ERROR
)]
fn a_failure_answers_with_its_own_status(
    #[case] error: ShadowResultsRouteError,
    #[case] status: StatusCode,
) {
    let response = failure(&error);
    assert_eq!(response.status(), status);
}
