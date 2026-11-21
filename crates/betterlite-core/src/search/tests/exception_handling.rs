#![cfg(test)]

use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_test_ports::refused_port::RefusedPort;

use super::error::SearchCallError;
use super::support::{Harness, Recorder, Registry, TestError, request, status_of};
use crate::api::search_http::SearchHttpError;

#[tokio::test]
async fn exception_handling_maps_a_provider_status_and_keeps_its_headers() {
    let served = serve(
        reply("401 Unauthorized", &[("x-request-id", "req-9")], "bad key"),
        Duration::ZERO,
    )
    .expect("the fixture serves");
    let registry = Registry::new(Recorder::default());
    let mut harness = Harness::new();
    let failure = harness
        .search(
            &request(r#""rust""#, &format!("http://{}", served.address)),
            &registry,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        &failure.error,
        SearchCallError::Http(SearchHttpError::Provider { provider }) if provider.status_code == 401
    ));
    assert!(matches!(failure.class, ErrorClass::Authentication(_)));
    let metadata = failure.class.metadata();
    assert_eq!(metadata.status_code, 401);
    assert_eq!(metadata.model.as_deref(), Some("brave/search"));
    assert_eq!(metadata.provider.as_deref(), Some("brave"));
    assert!(
        failure
            .response_headers
            .contains(&("x-request-id".to_owned(), "req-9".to_owned()))
    );
    assert_eq!(failure.to_string(), "bad key");
    served.finish().expect("the fixture answers");
}

#[tokio::test]
async fn exception_handling_maps_a_real_server_error_by_its_status() {
    let served = serve(
        reply("500 Internal Server Error", &[], "upstream broke"),
        Duration::ZERO,
    )
    .expect("the fixture serves");
    let registry = Registry::new(Recorder::default());
    let mut harness = Harness::new();
    let failure = harness
        .search(
            &request(r#""rust""#, &format!("http://{}", served.address)),
            &registry,
        )
        .await
        .unwrap_err();
    assert!(matches!(failure.class, ErrorClass::InternalServer(_)));
    assert_eq!(status_of(&failure.class), 500);
    served.finish().expect("the fixture answers");
}

#[tokio::test]
async fn exception_handling_keeps_a_synthesized_status_out_of_the_status_map() {
    let refused = RefusedPort::bind().expect("the refused port binds");
    let registry = Registry::new(Recorder::default());
    let mut harness = Harness::new();
    let failure = harness
        .search(
            &request(r#""rust""#, &format!("http://{}", refused.address)),
            &registry,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        &failure.error,
        SearchCallError::Http(SearchHttpError::Provider { provider }) if provider.status_code_is_synthesized
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert!(failure.response_headers.is_empty());
}

#[tokio::test]
async fn exception_handling_wraps_a_hook_failure_as_a_value_error() {
    let served =
        serve(reply("200 OK", &[], "not json"), Duration::ZERO).expect("the fixture serves");
    let registry = Registry::new(Recorder::default());
    let mut harness = Harness::new();
    let failure = harness
        .search(
            &request(r#""rust""#, &format!("http://{}", served.address)),
            &registry,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        failure.error,
        SearchCallError::Http(SearchHttpError::Hook(TestError::Decode(_)))
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(status_of(&failure.class), 500);
    assert!(failure.response_headers.is_empty());
    assert_eq!(failure.to_string(), "the reply is not a search response");
    served.finish().expect("the fixture answers");
}
