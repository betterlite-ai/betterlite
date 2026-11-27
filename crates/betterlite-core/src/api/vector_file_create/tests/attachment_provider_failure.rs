use betterlite_error::envelope::WireField;

use super::support::{Fixture, REQUEST_TIMEOUT, Spy, Which, attempt};
use crate::api::vector_store_search::error::SdkFailure;

const UNAUTHORIZED: &str =
    r#"{"error":{"message":"bad","type":"invalid_api_key","param":null,"code":"invalid_api_key"}}"#;

fn assert_authentication(failure: &SdkFailure, spy: &Spy) {
    assert_eq!(failure.class, "AuthenticationError");
    assert_eq!(failure.status, 401);
    assert_eq!(failure.kind, WireField::Null);
    assert_eq!(failure.param, WireField::Null);
    assert_eq!(failure.code, WireField::Null);
    assert_eq!(failure.message, UNAUTHORIZED);
    assert_eq!(spy.call_count(), 1);
}

#[tokio::test]
async fn attachment_provider_failure() {
    let fixture = Fixture::new();
    let create_spy = Spy::failing(401, UNAUTHORIZED);
    let create_failure = attempt(Which::Create, &create_spy, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect_err("the attachment call fails");
    assert_authentication(&create_failure, &create_spy);
    let list_spy = Spy::failing(401, UNAUTHORIZED);
    let list_failure = attempt(Which::List, &list_spy, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect_err("the attachment call fails");
    assert_authentication(&list_failure, &list_spy);
}

#[tokio::test]
async fn attachment_provider_failure_async() {
    let fixture = Fixture::new();
    let acreate_spy = Spy::failing(401, UNAUTHORIZED);
    let acreate_failure = attempt(
        Which::Acreate,
        &acreate_spy,
        &fixture,
        None,
        REQUEST_TIMEOUT,
    )
    .await
    .expect_err("the attachment call fails");
    assert_authentication(&acreate_failure, &acreate_spy);
    let alist_spy = Spy::failing(401, UNAUTHORIZED);
    let alist_failure = attempt(Which::Alist, &alist_spy, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect_err("the attachment call fails");
    assert_authentication(&alist_failure, &alist_spy);
}
