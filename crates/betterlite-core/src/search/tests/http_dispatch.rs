#![cfg(test)]

use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::search::provider::SearchQuery;
use rstest::rstest;

use super::error::SearchCallError;
use super::support::{ANSWER, Call, Harness, Recorder, Registry, object, request, status_of};
use crate::api::search_http::SearchHttpError;

#[tokio::test]
async fn http_dispatch_hands_the_handler_the_prepared_call() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}", served.address);
    let complete = format!("{base}/v1/search");
    let recorder = Recorder::default();
    let registry = Registry::new(recorder.clone());
    let mut search = request(r#"["rust","go"]"#, &base);
    search.kwargs = serde_json::from_str(r#"{"safesearch":"off"}"#).unwrap();
    let mut harness = Harness::new();
    harness.search(&search, &registry).await.unwrap();
    let validated = Headers::from([("x-validated".to_owned(), "yes".to_owned())]);
    let calls = recorder.calls();
    assert_eq!(
        calls.get(2..),
        Some(
            [
                Call::Validate {
                    headers: validated.clone(),
                    api_key: Some("sk-search".to_owned()),
                    api_base: Some(complete.clone()),
                },
                Call::Transform {
                    query: SearchQuery::Multiple(vec!["rust".to_owned(), "go".to_owned()]),
                    params: object(r#"{"safesearch":"off"}"#),
                    headers: validated,
                },
                Call::Url {
                    api_base: Some(complete),
                    params: object(r#"{"safesearch":"off"}"#),
                    has_body: true,
                    api_key: Some("sk-search".to_owned()),
                },
            ]
            .as_slice()
        )
    );
    let sent = served.finish().expect("the fixture answers");
    assert_eq!(sent.line, "POST /v1/search HTTP/1.1");
    assert_eq!(sent.body, r#"["rust","go"]"#);
}

#[rstest]
#[case::explicit_timeout(Some(Duration::from_millis(100)), 5.0_f64)]
#[case::zero_takes_the_default(Some(Duration::ZERO), 0.1_f64)]
#[case::absent_takes_the_default(None, 0.1_f64)]
#[tokio::test]
async fn http_dispatch_times_out_on_the_resolved_timeout(
    #[case] timeout: Option<Duration>,
    #[case] default_seconds: f64,
) {
    let served =
        serve(reply("200 OK", &[], ANSWER), Duration::from_secs(1)).expect("the fixture serves");
    let registry = Registry::new(Recorder::default());
    let mut search = request(r#""rust""#, &format!("http://{}", served.address));
    search.timeout = timeout;
    let mut harness = Harness::new();
    harness.core.request_timeout_seconds = default_seconds;
    let failure = harness.search(&search, &registry).await.unwrap_err();
    assert!(matches!(
        failure.error,
        SearchCallError::Http(SearchHttpError::Provider { .. })
    ));
    assert!(matches!(failure.class, ErrorClass::Timeout(_)));
    assert_eq!(status_of(&failure.class), 408);
    served.finish().expect("the fixture answers");
}

#[tokio::test]
async fn http_dispatch_prefers_an_explicit_timeout_over_the_default() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::from_millis(300))
        .expect("the fixture serves");
    let registry = Registry::new(Recorder::default());
    let mut search = request(r#""rust""#, &format!("http://{}", served.address));
    search.timeout = Some(Duration::from_secs(5));
    let mut harness = Harness::new();
    harness.core.request_timeout_seconds = 0.1_f64;
    harness.search(&search, &registry).await.unwrap();
    served.finish().expect("the fixture answers");
}

#[tokio::test]
async fn http_dispatch_rejects_a_default_timeout_that_is_no_duration() {
    let recorder = Recorder::default();
    let registry = Registry::new(recorder.clone());
    let mut harness = Harness::new();
    harness.core.request_timeout_seconds = -1.0_f64;
    let failure = harness
        .search(&request(r#""rust""#, "http://127.0.0.1:9"), &registry)
        .await
        .unwrap_err();
    assert!(matches!(
        failure.error,
        SearchCallError::Timeout { seconds, .. } if seconds < 0.0_f64
    ));
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(recorder.calls().len(), 2);
    assert_eq!(harness.state.model, "brave/search");
}
