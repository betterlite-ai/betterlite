use super::super::service::Operation;
use super::support::{
    Fixture, REQUEST_TIMEOUT, Resolution, Spy, call_delete, call_update, deleted, kept_headers,
    lost_query, replacement_body, success,
};
use crate::api::vector_store_http_write::params::UpdateParams;

// The writes hold only the service, so neither extra_query nor a registry can be reached from them
#[tokio::test]
async fn write_ignores_extra_query() {
    let spy = Spy::replying(success());
    let resolution = Resolution::new();
    let mut fixture = Fixture::new();
    fixture.extra_query = Some(lost_query());
    fixture.extra_headers = Some(kept_headers());
    fixture.extra_body = Some(replacement_body());
    let value = call_update(&spy, &fixture, &resolution, REQUEST_TIMEOUT)
        .await
        .expect("the mock answer is returned");
    assert_eq!(value, success());
    let seen = spy.seen().expect("the service saw the write");
    assert_eq!(seen.extra_headers, Some(kept_headers()));
    assert_eq!(seen.extra_body, Some(replacement_body()));
    assert_eq!(seen.optional_params, UpdateParams::default());
    assert_eq!(spy.call_count(), 1);
}

#[tokio::test]
async fn delete_ignores_extra_query_and_update_parameters() {
    let spy = Spy::replying(deleted());
    let resolution = Resolution::new();
    let mut fixture = Fixture::new();
    fixture.extra_query = Some(lost_query());
    fixture.extra_headers = Some(kept_headers());
    let value = call_delete(&spy, &fixture, &resolution, REQUEST_TIMEOUT)
        .await
        .expect("the mock answer is returned");
    assert_eq!(value, deleted());
    let seen = spy.seen().expect("the service saw the write");
    assert_eq!(seen.operation, Operation::Delete);
    assert_eq!(seen.optional_params, UpdateParams::default());
    assert_eq!(seen.extra_body, None);
    assert_eq!(seen.extra_headers, Some(kept_headers()));
    assert_eq!(spy.call_count(), 1);
}
