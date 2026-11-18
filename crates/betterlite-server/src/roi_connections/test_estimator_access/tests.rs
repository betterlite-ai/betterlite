#![cfg(test)]

use rstest::rstest;

use crate::roi_connections::error::RoiConnectionsError;
use crate::roi_connections::gateway_key::MISSING_KEY;
use crate::roi_connections::test_estimator_access::{
    CANNOT_ACCESS, CANNOT_CONNECT, test_estimator_access,
};
use crate::roi_connections::tests::support::{Answer, Wire, gateway, secret, settings};

#[tokio::test]
async fn test_estimator_access_accepts_a_listed_model() {
    let wire = Wire::new(vec![Answer::ok(r#"{"data":[{"id":"gpt-4o"}]}"#)]);
    test_estimator_access(&settings(), None, &gateway(&wire))
        .await
        .expect("access");
    let seen = wire.seen();
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].url, "http://litellm.internal/v1/models");
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some("Bearer sk-estimator")
    );
}

#[tokio::test]
async fn test_estimator_access_rejects_a_missing_model() {
    let wire = Wire::new(vec![Answer::ok(r#"{"data":[{"id":"other"}]}"#)]);
    let error = test_estimator_access(&settings(), None, &gateway(&wire))
        .await
        .err()
        .expect("no access");
    assert!(matches!(
        error,
        RoiConnectionsError::Conflict(CANNOT_ACCESS)
    ));
}

#[tokio::test]
async fn test_estimator_access_maps_an_error_status() {
    let wire = Wire::new(vec![Answer::status(401)]);
    let error = test_estimator_access(&settings(), None, &gateway(&wire))
        .await
        .err()
        .expect("no access");
    assert!(matches!(
        error,
        RoiConnectionsError::Conflict(CANNOT_CONNECT)
    ));
}

#[rstest]
#[case::not_json("not json")]
#[case::missing_data("{}")]
#[case::data_not_a_list(r#"{"data":{"id":"gpt-4o"}}"#)]
#[tokio::test]
async fn test_estimator_access_maps_an_invalid_answer(#[case] body: &str) {
    let wire = Wire::new(vec![Answer::ok(body)]);
    let error = test_estimator_access(&settings(), None, &gateway(&wire))
        .await
        .err()
        .expect("no access");
    assert!(matches!(
        error,
        RoiConnectionsError::Conflict(CANNOT_CONNECT)
    ));
}

#[tokio::test]
async fn test_estimator_access_without_a_credential_conflicts() {
    let mut roi = settings();
    roi.estimator_key = secret("");
    let wire = Wire::new(Vec::new());
    let error = test_estimator_access(&roi, None, &gateway(&wire))
        .await
        .err()
        .expect("no credential");
    assert!(matches!(error, RoiConnectionsError::Conflict(MISSING_KEY)));
    assert!(wire.seen().is_empty());
}

#[tokio::test]
async fn test_estimator_access_uses_the_master_key() {
    let mut roi = settings();
    roi.estimator_key = secret("");
    let master = secret("master-key");
    let wire = Wire::new(vec![Answer::ok(r#"{"data":[{"id":"gpt-4o"}]}"#)]);
    test_estimator_access(&roi, Some(&master), &gateway(&wire))
        .await
        .expect("access");
    let seen = wire.seen();
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer master-key"));
}
