#![cfg(test)]

use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_openai::openai_base_config::Object;

use super::SearchOptions;
use super::error::SearchCallError;
use super::support::{ANSWER, Call, Harness, Recorder, Registry, object, request, status_of};

fn url_calls(recorder: &Recorder) -> Vec<Call> {
    recorder
        .calls()
        .into_iter()
        .filter(|call| matches!(call, Call::Url { .. }))
        .collect()
}

#[tokio::test]
async fn url_resolution_starts_from_the_caller_base_without_a_body() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}", served.address);
    let recorder = Recorder::default();
    let registry = Registry::new(recorder.clone());
    let mut search = request(r#""rust""#, &base);
    search.options = SearchOptions {
        max_results: Some(3_i64),
        search_domain_filter: None,
        max_tokens_per_page: None,
        country: None,
    };
    let mut harness = Harness::new();
    harness.search(&search, &registry).await.unwrap();
    let complete = format!("{base}/v1/search");
    assert_eq!(
        url_calls(&recorder),
        [
            Call::Url {
                api_base: Some(base),
                params: object(r#"{"max_results":3}"#),
                has_body: false,
                api_key: Some("sk-search".to_owned()),
            },
            Call::Url {
                api_base: Some(complete),
                params: object(r#"{"max_results":3}"#),
                has_body: true,
                api_key: Some("sk-search".to_owned()),
            },
        ]
    );
    let sent = served.finish().expect("the fixture answers");
    assert_eq!(sent.line, "POST /v1/search HTTP/1.1");
}

#[tokio::test]
async fn url_resolution_passes_no_base_when_the_caller_sends_none() {
    let recorder = Recorder::new(true, Object::new());
    let registry = Registry::new(recorder.clone());
    let mut search = request(r#""rust""#, "http://127.0.0.1:9");
    search.api_base = None;
    let mut harness = Harness::new();
    harness.search(&search, &registry).await.unwrap_err();
    assert!(matches!(
        url_calls(&recorder).first(),
        Some(Call::Url { api_base: None, .. })
    ));
}

#[tokio::test]
async fn url_resolution_failure_stops_before_logging_and_sending() {
    let recorder = Recorder::new(true, Object::new());
    let registry = Registry::new(recorder.clone());
    let mut harness = Harness::new();
    let failure = harness
        .search(&request(r#""rust""#, "http://127.0.0.1:9"), &registry)
        .await
        .unwrap_err();
    assert!(matches!(failure.error, SearchCallError::Url(_)));
    assert_eq!(failure.to_string(), "the URL hook refused the call");
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(status_of(&failure.class), 500);
    assert_eq!(recorder.calls().len(), 2);
    assert_eq!(harness.state.model, "unset");
}
