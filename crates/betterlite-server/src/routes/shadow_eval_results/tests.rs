#![cfg(test)]

use std::sync::Arc;

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use rstest::rstest;

use super::{ResultsState, detail_route, failure, list_route};
use crate::shadow_eval_results::error::ShadowResultsRouteError;
use crate::shadow_eval_results::query::ShadowQueryError;
use crate::shadow_eval_validation::error::AccessError;

fn state() -> Arc<ResultsState> {
    Arc::new(ResultsState { store: None })
}

// the middleware answers an absent caller, so neither handler may run its own read first
#[tokio::test]
async fn an_unauthenticated_list_answers_401() {
    let response = list_route(State(state()), None, RawQuery(None)).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_unauthenticated_detail_answers_401() {
    let response = detail_route(State(state()), Path("job-1".to_owned()), None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
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
