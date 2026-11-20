use std::time::Duration;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use rstest::rstest;

use super::support::{IMPATIENT, ProxiedSource};
use super::*;
use crate::api::genai_http::error::GenAiHttpError;
use crate::api::vector_store_http_write::error::HeaderMap;

#[tokio::test]
async fn a_status_error_goes_through_the_shared_error_handler() {
    let events = Events::new();
    let wire = Wire::replying(
        "sync",
        Reply::json(429, ERROR_BODY)
            .with_header("retry-after", "3")
            .with_header("content-encoding", "gzip"),
        &events,
    );
    let source = Source::new(wire);
    let config = TestConfig::new("https://gemini.test");
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &events).await;

    let Err(GenAiHttpError::Provider(error)) = outcome else {
        panic!("a 429 is a provider error");
    };
    assert_eq!(error.status_code, 429);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, ERROR_BODY);
    let length = ERROR_BODY.len().to_string();
    assert_eq!(
        error.headers,
        HeaderMap::from([
            ("retry-after".to_owned(), DecisionJson::Text("3".to_owned())),
            ("content-length".to_owned(), DecisionJson::Text(length)),
        ])
    );
    assert_eq!(config.errors().len(), 1);
    assert!(fixture.state.httpx_response.is_none());
}

#[tokio::test]
async fn an_error_body_that_is_empty_or_not_utf8_keeps_its_shape() {
    let events = Events::new();
    let config = TestConfig::new("https://gemini.test");
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");
    {
        let wire = Wire::replying(
            "sync",
            Reply::json(500, "").with_header("content-length", "0"),
            &events,
        );
        let source = Source::new(wire);
        let deps = GenerateContentDeps {
            config: &config,
            source: &source,
            supplied: None,
            stream: StreamSettings::default(),
        };
        let Err(GenAiHttpError::Provider(error)) = fixture.sync_call(&deps, &target, &events).await
        else {
            panic!("a 500 is a provider error");
        };
        assert_eq!(error.status_code, 500);
        assert!(error.headers.is_empty());
        assert!(error.message.is_empty());
    }
    {
        let mut failure = Reply::json(500, "");
        failure.body = vec![0xff, 0xfe];
        let wire = Wire::replying("sync", failure, &events);
        let source = Source::new(wire);
        let deps = GenerateContentDeps {
            config: &config,
            source: &source,
            supplied: None,
            stream: StreamSettings::default(),
        };
        let Err(GenAiHttpError::Provider(error)) = fixture.sync_call(&deps, &target, &events).await
        else {
            panic!("a 500 is a provider error");
        };
        assert_eq!(error.message, "2 bytes that are not UTF-8");
        assert_eq!(
            error.headers,
            HeaderMap::from([(
                "content-length".to_owned(),
                DecisionJson::Text("2".to_owned()),
            )])
        );
    }
}

#[tokio::test]
async fn a_transport_failure_becomes_a_synthesized_500() {
    let events = Events::new();
    let wire = Wire::failing("sync", TransportError::Closed, &events);
    let source = Source::new(wire);
    let config = TestConfig::new("https://gemini.test");
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &events).await;

    let Err(GenAiHttpError::Provider(error)) = outcome else {
        panic!("a transport failure is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(error.message, TransportError::Closed.to_string());
    assert!(error.headers.is_empty());
}

#[rstest]
#[case::no_timeout(None, "litellm.Timeout: Connection timed out after None seconds.")]
#[case::with_timeout(
    Some(Duration::from_millis(100)),
    "litellm.Timeout: Connection timed out after 0.1 seconds."
)]
#[tokio::test]
async fn a_sync_timeout_carries_408_and_the_timeout_it_was_given(
    #[case] timeout: Option<Duration>,
    #[case] message: &str,
) {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], REPLY),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let source = ProxiedSource::new(IMPATIENT);
    let config = TestConfig::new(&format!("http://{}", serving.address));
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, timeout);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &Events::new()).await;

    let Err(GenAiHttpError::Provider(error)) = outcome else {
        panic!("a timeout is a provider error");
    };
    assert_eq!(error.status_code, 408);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, message);
    assert!(error.headers.is_empty());
}

#[tokio::test]
async fn an_async_timeout_carries_408_and_the_client_timeout() {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], REPLY),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let source = ProxiedSource::new(IMPATIENT);
    let config = TestConfig::new(&format!("http://{}", serving.address));
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("agenerate_content");

    let outcome = fixture.async_call(Events::new(), &deps, &target).await;

    let Err(GenAiHttpError::Provider(error)) = outcome else {
        panic!("a timeout is a provider error");
    };
    assert_eq!(error.status_code, 408);
    assert!(!error.status_code_is_synthesized);
    assert!(
        error
            .message
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken="),
        "the client's own read timeout is reported: {}",
        error.message
    );
    assert!(error.message.ends_with(" seconds"));
    assert!(error.headers.is_empty());
}

#[tokio::test]
async fn a_transform_failure_is_returned_as_the_provider_error() {
    let events = Events::new();
    let wire = Wire::replying("sync", Reply::json(200, REPLY), &events);
    let source = Source::new(wire);
    let config = TestConfig::new("https://gemini.test").failing_transform();
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &events).await;

    assert!(matches!(outcome, Err(GenAiHttpError::Transform(_))));
    assert!(events.recorded().is_empty());
}

#[tokio::test]
async fn an_auth_failure_is_returned_as_the_provider_error() {
    let events = Events::new();
    let wire = Wire::replying("sync", Reply::json(200, REPLY), &events);
    let source = Source::new(wire);
    let config = TestConfig::new("https://gemini.test").failing_auth();
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &events).await;

    assert!(matches!(outcome, Err(GenAiHttpError::Auth(_))));
    assert!(events.recorded().is_empty());
}

#[tokio::test]
async fn a_body_that_is_not_an_object_is_reported() {
    let events = Events::new();
    let wire = Wire::replying("sync", Reply::json(200, REPLY), &events);
    let source = Source::new(wire);
    let config = TestConfig::new("https://gemini.test").with_text_body();
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &events).await;

    assert!(matches!(outcome, Err(GenAiHttpError::BodyNotObject)));
}

#[tokio::test]
async fn a_reply_the_provider_cannot_decode_is_returned_as_the_provider_error() {
    let events = Events::new();
    let wire = Wire::replying("sync", Reply::json(200, "not json"), &events);
    let source = Source::new(wire);
    let config = TestConfig::new("https://gemini.test").failing_decode();
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &events).await;

    let Err(GenAiHttpError::Provider(error)) = outcome else {
        panic!("a decode failure is a provider error");
    };
    assert_eq!(error.status_code, 200);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(config.errors().len(), 1);
    assert!(fixture.state.httpx_response.is_none());
}

#[tokio::test]
async fn an_async_open_stream_for_a_call_that_did_not_stream_is_reported() {
    let events = Events::new();
    let wire = Wire::streaming("async", Reply::json(200, SSE), &events).always_streaming();
    let source = Source::new(wire);
    let config = TestConfig::new("https://gemini.test");
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("agenerate_content");

    let outcome = fixture.async_call(events.clone(), &deps, &target).await;

    assert!(matches!(outcome, Err(GenAiHttpError::UnexpectedStream)));
}

#[tokio::test]
async fn an_open_stream_for_a_call_that_did_not_stream_is_reported() {
    let events = Events::new();
    let wire = Wire::streaming("sync", Reply::json(200, SSE), &events).always_streaming();
    let source = Source::new(wire);
    let config = TestConfig::new("https://gemini.test");
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &events).await;

    assert!(matches!(outcome, Err(GenAiHttpError::UnexpectedStream)));
}

#[tokio::test]
async fn a_sync_stream_that_receives_an_open_stream_is_reported() {
    let events = Events::new();
    let wire = Wire::streaming("sync", Reply::json(200, SSE), &events).always_streaming();
    let source = Source::new(wire);
    let config = TestConfig::new("https://gemini.test");
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, true, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &events).await;

    assert!(matches!(outcome, Err(GenAiHttpError::UnexpectedStream)));
}

#[tokio::test]
async fn a_400_reply_is_a_provider_error() {
    let events = Events::new();
    let wire = Wire::replying("sync", Reply::json(400, ERROR_BODY), &events);
    let source = Source::new(wire);
    let config = TestConfig::new("https://gemini.test");
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &events).await;

    let Err(GenAiHttpError::Provider(error)) = outcome else {
        panic!("the smallest error status is a provider error");
    };
    assert_eq!(error.status_code, 400);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, ERROR_BODY);
}

#[rstest]
#[case::client_error("400 Bad Request", 400_u16)]
#[case::server_error("500 Internal Server Error", 500_u16)]
#[tokio::test]
async fn a_streaming_call_with_an_error_reply_reports_the_provider_error(
    #[case] status: &str,
    #[case] code: u16,
) {
    let serving = serve(
        reply(status, &[("content-type", "application/json")], ERROR_BODY),
        Duration::ZERO,
    )
    .expect("the fixture serves");
    let source = ProxiedSource::new(IMPATIENT);
    let config = TestConfig::new(&format!("http://{}", serving.address));
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, true, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &Events::new()).await;

    let Err(GenAiHttpError::Provider(error)) = outcome else {
        panic!("an error status is a provider error even when the call streams");
    };
    assert_eq!(error.status_code, code);
    assert_eq!(error.message, ERROR_BODY);
}

#[tokio::test]
async fn a_unary_call_through_the_pooled_client_returns_the_response() {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], REPLY),
        Duration::ZERO,
    )
    .expect("the fixture serves");
    let source = ProxiedSource::new(TransportLimits::DEFAULT);
    let config = TestConfig::new(&format!("http://{}", serving.address));
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &Events::new()).await;

    let Ok(GenerateContentOutcome::Response(response)) = outcome else {
        panic!("a 200 reply through the pooled client is a response");
    };
    assert_eq!(response, json(REPLY));
    assert!(fixture.state.httpx_response.is_some());
}

#[rstest]
#[case::client_error("400 Bad Request", 400_u16)]
#[case::server_error("500 Internal Server Error", 500_u16)]
#[tokio::test]
async fn an_async_stream_that_receives_an_error_reply_reports_the_provider_error(
    #[case] status: &str,
    #[case] code: u16,
) {
    let serving = serve(
        reply(status, &[("content-type", "application/json")], ERROR_BODY),
        Duration::ZERO,
    )
    .expect("the fixture serves");
    let source = ProxiedSource::new(TransportLimits::DEFAULT);
    let config = TestConfig::new(&format!("http://{}", serving.address));
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, true, None);
    let mut fixture = Fixture::new("agenerate_content");

    let outcome = fixture.async_call(Events::new(), &deps, &target).await;

    let Err(GenAiHttpError::Provider(error)) = outcome else {
        panic!("an error status is a provider error even when the call streams");
    };
    assert_eq!(error.status_code, code);
    assert_eq!(error.message, ERROR_BODY);
}

#[tokio::test]
async fn a_request_header_that_is_not_a_header_name_is_reported() {
    let events = Events::new();
    let wire = Wire::replying("sync", Reply::json(200, REPLY), &events);
    let source = Source::new(wire);
    let config = TestConfig::new("https://gemini.test");
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let mut headers = IndexMap::new();
    headers.insert("bad name".to_owned(), "value".to_owned());
    let mut target = call(&contents, &generation, &params, false, None);
    target.extra_headers = Some(&headers);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture.sync_call(&deps, &target, &events).await;

    let Err(GenAiHttpError::Provider(error)) = outcome else {
        panic!("a header the wire cannot carry is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert!(
        error
            .message
            .starts_with("the bad name header name is not a header name"),
        "the offending header is named: {}",
        error.message
    );
    assert_eq!(events.recorded(), vec!["log".to_owned()]);
}
