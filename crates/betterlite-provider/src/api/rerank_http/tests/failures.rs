use std::time::Duration;

use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;

use super::super::client::ClientKind;
use super::super::error::RerankHttpError;
use super::{Bench, ConfigError, Fault, HeaderMap, Refusal, Reply, RerankLogging, supplied_sent};

// LiteLLM lets httpx reject a bad URL or header and turns the exception into a synthesized 500 provider error
pub(super) async fn before_the_send() {
    let Err(RerankHttpError::Config(ConfigError::Validate)) = refused(Refusal::Validate).await
    else {
        panic!("validate refuses before the send");
    };
    let Err(RerankHttpError::Config(ConfigError::Transform)) = refused(Refusal::Transform).await
    else {
        panic!("transform refuses before the send");
    };
    let Err(RerankHttpError::Logging(PrePostError::RawRequestCaptured)) =
        refused(Refusal::PreCall).await
    else {
        panic!("the pre-call refuses before the send");
    };
    let Err(RerankHttpError::RequestUrl { .. }) = refused(Refusal::Url).await else {
        panic!("a bad URL is a typed error");
    };
    let Err(RerankHttpError::HeaderName { name, .. }) = refused(Refusal::HeaderName).await else {
        panic!("a bad header name is a typed error");
    };
    assert_eq!(name, "x bad");
    let Err(RerankHttpError::HeaderValue { .. }) = refused(Refusal::HeaderValue).await else {
        panic!("a bad header value is a typed error");
    };
}

async fn refused(refusal: Refusal) -> Result<RerankResponse, RerankHttpError<ConfigError>> {
    let mut bench = Bench::refusing(refusal);
    let mut logging = RerankLogging::default();
    let outcome = bench.run(Json::Bool(false), None, &mut logging).await;
    assert_eq!(supplied_sent(&bench).len(), refusal.sent());
    assert!(bench.config.errors().is_empty());
    outcome
}

#[tokio::test]
async fn rerank_http_provider_failure() {
    let mut bench = Bench::new();
    bench.supplied.reply = Reply::json(429, "slow").with_header("retry-after", "2");
    let mut logging = RerankLogging::default();
    let outcome = bench.run(Json::Bool(false), None, &mut logging).await;
    let Err(RerankHttpError::Provider(error)) = outcome else {
        panic!("the 429 becomes the provider error the config builds");
    };
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "rerank:slow");
    let expected = HeaderMap::from([
        ("content-length".to_owned(), Json::Text(String::from("4"))),
        ("retry-after".to_owned(), Json::Text(String::from("2"))),
    ]);
    assert_eq!(error.headers, expected);
    assert_eq!(
        bench.config.errors(),
        vec![(String::from("slow"), 429, expected)]
    );
    assert_eq!(supplied_sent(&bench).len(), 1);
    assert_eq!(
        bench.events.recorded(),
        vec!["validate", "url", "transform", "pre_call", "post"]
    );
    assert!(matches!(
        refused(Refusal::Response).await,
        Err(RerankHttpError::Config(ConfigError::Response))
    ));
    send_failure_conversion().await;
}

async fn send_failure_conversion() {
    let mut bench = Bench::new();
    bench.supplied.fail(Fault::Send);
    let mut logging = RerankLogging::default();
    let outcome = bench.run(Json::Bool(false), None, &mut logging).await;
    let Err(RerankHttpError::Provider(error)) = outcome else {
        panic!("a failed send becomes the provider error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(
        error.message,
        "rerank:the response body is larger than 1024 bytes"
    );
    assert_eq!(
        bench.config.errors(),
        vec![(
            String::from("the response body is larger than 1024 bytes"),
            500,
            HeaderMap::new()
        )]
    );
    assert!(
        !bench
            .events
            .recorded()
            .contains(&String::from("response_transform"))
    );
}

async fn timeout_conversion(is_async: Json, expected: &str) {
    let mut bench = Bench::new();
    bench.supplied_kind = if is_async == Json::Bool(true) {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    };
    bench.supplied.fail(Fault::Timeout);
    let mut logging = RerankLogging::default();
    let outcome = bench
        .run(is_async, Some(Duration::from_secs(5)), &mut logging)
        .await;
    let Err(RerankHttpError::Provider(error)) = outcome else {
        panic!("a timeout becomes the provider error");
    };
    assert_eq!(error.status_code, 408);
    assert!(error.message.starts_with(expected));
}

#[tokio::test]
async fn rerank_http_timeout_messages() {
    timeout_conversion(
        Json::Bool(false),
        "rerank:litellm.Timeout: Connection timed out after 5 seconds.",
    )
    .await;
    timeout_conversion(
        Json::Bool(true),
        "rerank:litellm.Timeout: Connection timed out. Timeout passed=5, time taken=",
    )
    .await;
}
