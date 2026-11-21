#![cfg(test)]

use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;

use super::error::SearchCallError;
use super::support::{ANSWER, Harness, Recorder, Registry, request, status_of};

#[tokio::test]
async fn provider_lookup_rejects_a_provider_without_a_search_config() {
    let recorder = Recorder::default();
    let registry = Registry::new(recorder.clone());
    let mut unknown = request(r#""rust""#, "http://127.0.0.1:9");
    unknown.search_provider = "nowhere".to_owned();
    let mut harness = Harness::new();
    let failure = harness.search(&unknown, &registry).await.unwrap_err();
    assert!(matches!(
        &failure.error,
        SearchCallError::Unsupported { provider } if provider == "nowhere"
    ));
    assert_eq!(
        failure.to_string(),
        "Search is not supported for provider: nowhere"
    );
    assert_eq!(registry.asked(), ["nowhere"]);
    assert!(recorder.calls().is_empty());
    assert!(matches!(failure.class, ErrorClass::ApiConnection(_)));
    assert_eq!(status_of(&failure.class), 500);
    let metadata = failure.class.metadata();
    assert_eq!(metadata.model.as_deref(), Some("nowhere/search"));
    assert_eq!(metadata.provider.as_deref(), Some("nowhere"));
    assert_eq!(harness.state.model, "unset");
}

#[tokio::test]
async fn provider_lookup_runs_the_config_it_returns() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let recorder = Recorder::default();
    let registry = Registry::new(recorder.clone());
    let mut harness = Harness::new();
    harness
        .search(
            &request(r#""rust""#, &format!("http://{}", served.address)),
            &registry,
        )
        .await
        .unwrap();
    assert_eq!(registry.asked(), ["brave"]);
    assert_eq!(recorder.calls().len(), 5);
    served.finish().expect("the fixture answers");
}
