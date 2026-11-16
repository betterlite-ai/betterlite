use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::Principal;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{
    DetailMessageEnvelope, admin, call, decode, internal, spend_store, state, store_with,
};

#[rstest]
#[case::global_viewer(admin())]
#[case::internal_viewer(internal("u1"))]
#[tokio::test]
async fn models_reject_a_negative_limit(#[case] identity: Principal) {
    let store = store_with(spend_store());
    let reply = call(
        "/global/spend/models?limit=-5",
        Some(&identity),
        state(Some(&store), None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let body: DetailMessageEnvelope = decode(&reply);
    assert_eq!(
        body.detail.error,
        "Invalid limit: -5, error: Limit must be greater than 0"
    );
    assert!(store.calls().is_empty());
}

#[tokio::test]
async fn models_report_no_database_before_a_negative_limit() {
    let reply = call(
        "/global/spend/models?limit=-5",
        Some(&admin()),
        state(None, None, Decimal::ZERO),
    )
    .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: DetailMessageEnvelope = decode(&reply);
    assert_eq!(body.detail.error, "No db connected");
}
