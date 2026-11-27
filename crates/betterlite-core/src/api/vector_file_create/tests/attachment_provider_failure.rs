use betterlite_error::envelope::WireField;

use super::support::{Fixture, Spy, Which, refused};
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
    let create_failure = refused(Which::Create, &create_spy, &fixture, None).await;
    assert_authentication(&create_failure, &create_spy);
    let list_spy = Spy::failing(401, UNAUTHORIZED);
    let list_failure = refused(Which::List, &list_spy, &fixture, None).await;
    assert_authentication(&list_failure, &list_spy);
}

#[tokio::test]
async fn attachment_provider_failure_async() {
    let fixture = Fixture::new();
    let acreate_spy = Spy::failing(401, UNAUTHORIZED);
    let acreate_failure = refused(Which::Acreate, &acreate_spy, &fixture, None).await;
    assert_authentication(&acreate_failure, &acreate_spy);
    let alist_spy = Spy::failing(401, UNAUTHORIZED);
    let alist_failure = refused(Which::Alist, &alist_spy, &fixture, None).await;
    assert_authentication(&alist_failure, &alist_spy);
}
