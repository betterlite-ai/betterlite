#![cfg(test)]

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::{OriginalUri, Path, RawQuery, State};
use axum::http::{Request, StatusCode, Uri};
use axum::response::IntoResponse as _;
use betterlite_auth::auth_identity::records::Principal;
use rstest::rstest;
use tower::ServiceExt as _;

use super::{ResultsState, answered, failure, routes, shadow_eval_read};
use crate::auth::auth_identity::AuthContext;
use crate::routes::shadow_eval_lifecycle::routes as lifecycle_routes;
use crate::routes::tests::{empty_config, state_for};
use crate::shadow_eval_results::error::ShadowResultsRouteError;
use crate::shadow_eval_results::query::ShadowQueryError;
use crate::shadow_eval_validation::error::AccessError;

fn state() -> Arc<ResultsState> {
    Arc::new(ResultsState { store: None })
}

// the middleware answers an absent caller, so the handler's identity check must run before any read
#[tokio::test]
async fn an_unauthenticated_list_answers_401() {
    let error = shadow_eval_read(
        State(state()),
        OriginalUri(Uri::from_static("/auto_router/shadow_eval")),
        None,
        None,
        RawQuery(None),
    )
    .await
    .unwrap_err();
    assert_eq!(error.into_response().status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_unauthenticated_detail_answers_401() {
    let error = shadow_eval_read(
        State(state()),
        OriginalUri(Uri::from_static("/auto_router/shadow_eval/job-1")),
        Some(Path("job-1".to_owned())),
        None,
        RawQuery(None),
    )
    .await
    .unwrap_err();
    assert_eq!(error.into_response().status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_unauthenticated_start_answers_401() {
    let error = shadow_eval_read(
        State(state()),
        OriginalUri(Uri::from_static("/auto_router/shadow_eval/start")),
        None,
        None,
        RawQuery(None),
    )
    .await
    .unwrap_err();
    assert_eq!(error.into_response().status(), StatusCode::UNAUTHORIZED);
}

// LiteLLM's {job_id} route reads GET on the static start path as job "start": the merged router must reach the detail read, not the POST route's 405 or the list read's 422
#[tokio::test]
async fn get_on_the_start_path_reaches_the_detail_read() {
    let state = state_for(&empty_config());
    let router = routes(&state).merge(lifecycle_routes(&state));
    let mut request = Request::builder()
        .uri("/auto_router/shadow_eval/start?limit=zero")
        .body(Body::empty())
        .unwrap();
    // an internal user passes the identity check and the detail read refuses the role before its database check, while the list read would reject the query with 422 first
    let principal: Principal = serde_json::from_str(
        r#"{"principal_type":"human","subject":"user-1","auth_method":"api_key"}"#,
    )
    .expect("the internal user's principal");
    request.extensions_mut().insert(AuthContext { principal });
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// the listing path keeps the list read: the query is rejected with 422 before the role check, while the start path's detail read would answer 403
#[tokio::test]
async fn get_on_the_list_path_rejects_an_invalid_limit() {
    let state = state_for(&empty_config());
    let router = routes(&state).merge(lifecycle_routes(&state));
    let mut request = Request::builder()
        .uri("/auto_router/shadow_eval?limit=zero")
        .body(Body::empty())
        .unwrap();
    let principal: Principal = serde_json::from_str(
        r#"{"principal_type":"human","subject":"user-1","auth_method":"api_key"}"#,
    )
    .expect("the internal user's principal");
    request.extensions_mut().insert(AuthContext { principal });
    let response = router.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

// the read's success path reaches the caller as its JSON body, not as the empty 200 a default response carries
#[tokio::test]
async fn answered_serializes_the_read_into_the_json_body() {
    let Ok(response) = answered(Ok(vec!["job-1".to_owned()])) else {
        panic!("the read's success path answered with an error");
    };
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
    ShadowResultsRouteError::Query(ShadowQueryError::Invalid {
        parameter: "limit",
        kind: "int_parsing",
        msg: "Input should be a valid integer, unable to parse string as an integer"
    }),
    StatusCode::UNPROCESSABLE_ENTITY
)]
#[case::over_limit(
    ShadowResultsRouteError::Query(ShadowQueryError::TooLarge {
        bytes: 2,
        limit: 1
    }),
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

// LiteLLM's public_validation_errors keeps the pydantic type, loc and msg of each rejected parameter
#[tokio::test]
async fn an_invalid_query_answers_the_pydantic_validation_body() {
    let error = ShadowResultsRouteError::Query(ShadowQueryError::Invalid {
        parameter: "limit",
        kind: "int_parsing",
        msg: "Input should be a valid integer, unable to parse string as an integer",
    });
    let response = failure(&error);
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        String::from_utf8(body.to_vec()).unwrap(),
        r#"{"detail":[{"type":"int_parsing","loc":["query","limit"],"msg":"Input should be a valid integer, unable to parse string as an integer"}]}"#,
    );
}
