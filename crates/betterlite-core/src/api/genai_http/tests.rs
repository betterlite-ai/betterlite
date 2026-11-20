#![cfg(test)]

mod support;

use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json as DecisionJson;
use betterlite_prompts::arguments::json::{Json, Object};
use indexmap::IndexMap;
use rstest::rstest;

use self::support::{
    AuthRecord, Events, Fixture, IMPATIENT, ProxiedSource, Reply, Source, TestConfig, Wire, kwargs,
};
use crate::api::genai_http::error::GenAiHttpError;
use crate::api::genai_http::service::{
    AsyncGenerateContentOutcome, AsyncStream, GenerateContentCall, GenerateContentDeps,
    GenerateContentOutcome, SyncStream,
};
use crate::api::genai_http::stream::StreamSettings;
use crate::api::vector_store_http_write::error::HeaderMap;
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use crate::genai_setup::params::GenerateContentLiteLLMParams;

const MODEL: &str = "gemini-2.0-flash";
const CONTENTS: &str = r#"[{"role":"user","parts":[{"text":"hi"}]}]"#;
const CONFIG: &str = r#"{"temperature":0.2}"#;
const REPLY: &str = r#"{"candidates":[{"content":{"parts":[{"text":"hello"}],"role":"model"},"finishReason":"STOP"}]}"#;
const ERROR_BODY: &str = r#"{"error":{"code":429,"message":"quota"}}"#;
const SSE: &str = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hello\"}],\"role\":\"model\"}}]}\n\n";
const BODY: &str = r#"{"model":"gemini-2.0-flash","contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"temperature":0.2}}"#;
const UNARY_URL: &str = "https://gemini.test/v1beta/models/gemini-2.0-flash:generateContent";
const STREAM_URL: &str =
    "https://gemini.test/v1beta/models/gemini-2.0-flash:streamGenerateContent?alt=sse";

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

fn decision(text: &str) -> DecisionJson {
    serde_json::from_str(text).expect("the expected JSON parses")
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the expected JSON parses")
}

fn call<'call>(
    contents: &'call Json,
    config: &'call Object,
    params: &'call GenerateContentLiteLLMParams,
    stream: bool,
    timeout: Option<Duration>,
) -> GenerateContentCall<'call> {
    GenerateContentCall {
        model: MODEL,
        contents,
        config,
        tools: None,
        custom_llm_provider: "gemini",
        litellm_params: params,
        extra_headers: None,
        extra_body: None,
        system_instruction: None,
        timeout,
        stream,
    }
}

fn request_body(recorded: &support::Recorded) -> Json {
    serde_json::from_slice(&recorded.request.body).expect("the request body is JSON")
}

#[tokio::test]
async fn a_sync_unary_call_sends_the_authenticated_request_and_records_it() {
    let events = Events::new();
    let wire = Wire::replying(
        "sync",
        Reply::json(200, REPLY).with_header("x-request-id", "r1"),
        &events,
    );
    let source = Source::new(wire.clone());
    let config = TestConfig::new("https://gemini.test");
    let stream = StreamSettings::default();
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream,
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs(r#"{"api_base":"https://ignored.test","api_key":"sk-caller"}"#);
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture
        .sync_call(&deps, &target, &events)
        .await
        .expect("the call succeeds");

    let GenerateContentOutcome::Response(response) = outcome else {
        panic!("a unary call answers a response");
    };
    assert_eq!(response, json(REPLY));
    {
        let requests = wire.requests();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].streaming);
        assert_eq!(requests[0].request.url.as_str(), UNARY_URL);
        assert_eq!(
            requests[0]
                .request
                .headers
                .get("x-goog-api-key")
                .expect("the auth header travels"),
            "provider-key"
        );
        assert_eq!(
            requests[0]
                .request
                .headers
                .get("content-type")
                .expect("the content type travels"),
            "application/json"
        );
        assert_eq!(request_body(&requests[0]), json(BODY));
    }
    assert_eq!(
        config.auth_calls(),
        vec![AuthRecord {
            api_base: Some("https://ignored.test".to_owned()),
            model: MODEL.to_owned(),
            stream: false,
        }]
    );
    assert_eq!(
        fixture
            .state
            .httpx_response
            .as_ref()
            .map(|reply| reply.status),
        Some(200)
    );
    assert_eq!(fixture.input(), Some(&decision(CONTENTS)));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&decision(&format!("\"{UNARY_URL}\"")))
    );
    assert_eq!(fixture.logged("complete_input_dict"), Some(&decision(BODY)));
    assert_eq!(
        fixture.logged("headers"),
        Some(&decision(
            r#"{"Content-Type":"application/json","x-goog-api-key":"pr****ey"}"#
        ))
    );
}

#[tokio::test]
async fn an_async_unary_call_sends_the_authenticated_request() {
    let events = Events::new();
    let wire = Wire::replying("async", Reply::json(200, REPLY), &events);
    let source = Source::new(wire.clone());
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

    let outcome = fixture
        .async_call(events.clone(), &deps, &target)
        .await
        .expect("the call succeeds");

    let AsyncGenerateContentOutcome::Response(response) = outcome else {
        panic!("a unary call answers a response");
    };
    assert_eq!(response, json(REPLY));
    {
        let requests = wire.requests();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].streaming);
        assert_eq!(requests[0].request.url.as_str(), UNARY_URL);
    }
    assert!(!config.auth_calls()[0].stream);
    assert_eq!(
        fixture
            .state
            .httpx_response
            .as_ref()
            .map(|reply| reply.status),
        Some(200)
    );
}

#[tokio::test]
async fn pre_call_logging_finishes_before_the_transport_is_called() {
    let events = Events::new();
    let wire = Wire::replying("transport", Reply::json(200, REPLY), &events);
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

    fixture
        .sync_call(&deps, &target, &events)
        .await
        .expect("the call succeeds");

    assert_eq!(
        events.recorded(),
        vec!["log".to_owned(), "transport".to_owned()]
    );
}

#[tokio::test]
async fn the_callers_extra_headers_beat_the_provider_headers() {
    let events = Events::new();
    let wire = Wire::replying("sync", Reply::json(200, REPLY), &events);
    let source = Source::new(wire.clone());
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
    headers.insert("x-goog-api-key".to_owned(), "caller-key".to_owned());
    headers.insert("x-caller".to_owned(), "yes".to_owned());
    let mut target = call(&contents, &generation, &params, false, None);
    target.extra_headers = Some(&headers);
    let mut fixture = Fixture::new("generate_content");

    fixture
        .sync_call(&deps, &target, &events)
        .await
        .expect("the call succeeds");

    let requests = wire.requests();
    assert_eq!(
        requests[0]
            .request
            .headers
            .get("x-goog-api-key")
            .expect("the caller's key travels"),
        "caller-key"
    );
    assert_eq!(
        requests[0]
            .request
            .headers
            .get("x-caller")
            .expect("the caller's header travels"),
        "yes"
    );
    assert_eq!(
        fixture.logged("headers"),
        Some(&decision(
            r#"{"Content-Type":"application/json","x-goog-api-key":"ca****ey","x-caller":"yes"}"#
        ))
    );
}

#[tokio::test]
async fn the_callers_extra_body_beats_the_transformed_body() {
    let events = Events::new();
    let wire = Wire::replying("sync", Reply::json(200, REPLY), &events);
    let source = Source::new(wire.clone());
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
    let extra = object(r#"{"model":"overridden","safetySettings":[{"category":"HARM"}]}"#);
    let mut target = call(&contents, &generation, &params, false, None);
    target.extra_body = Some(&extra);
    let mut fixture = Fixture::new("generate_content");

    fixture
        .sync_call(&deps, &target, &events)
        .await
        .expect("the call succeeds");

    let requests = wire.requests();
    assert_eq!(
        request_body(&requests[0]),
        json(
            r#"{"model":"overridden","contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"temperature":0.2},"safetySettings":[{"category":"HARM"}]}"#
        )
    );
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&decision(
            r#"{"model":"overridden","contents":[{"role":"user","parts":[{"text":"hi"}]}],"generationConfig":{"temperature":0.2},"safetySettings":[{"category":"HARM"}]}"#
        ))
    );
}

#[tokio::test]
async fn a_sync_stream_returns_the_iterator_and_its_metadata_before_the_first_event() {
    let events = Events::new();
    let wire = Wire::streaming(
        "sync",
        Reply::json(200, SSE)
            .with_header("x-request-id", "r1")
            .with_header("x-ratelimit-remaining-requests", "9"),
        &events,
    );
    let source = Source::new(wire.clone());
    let config = TestConfig::new("https://gemini.test");
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: None,
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs(r#"{"model_info":{"id":"deployment-7"}}"#);
    let target = call(&contents, &generation, &params, true, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture
        .sync_call(&deps, &target, &events)
        .await
        .expect("the stream starts");

    let GenerateContentOutcome::Stream(stream) = outcome else {
        panic!("a streaming call answers an iterator");
    };
    assert_eq!(stream.metadata.model, MODEL);
    assert_eq!(
        stream.metadata.request_body.get("model"),
        Some(&decision(r#""gemini-2.0-flash""#))
    );
    assert_eq!(
        stream.metadata.hidden_params.get("model_id"),
        Some(&decision(r#""deployment-7""#))
    );
    assert_eq!(
        stream.metadata.hidden_params.get("api_base"),
        Some(&decision(&format!("\"{STREAM_URL}\"")))
    );
    assert_eq!(
        stream.metadata.hidden_params.get("cache_key"),
        Some(&decision(r#""""#))
    );
    assert_eq!(
        stream.metadata.hidden_params.get("response_cost"),
        Some(&decision(r#""""#))
    );
    assert_eq!(
        stream.metadata.hidden_params.get("additional_headers"),
        Some(&decision(
            r#"{"x-ratelimit-remaining-requests":"9","llm_provider-x-ratelimit-remaining-requests":"9","llm_provider-x-request-id":"r1"}"#
        ))
    );
    {
        let requests = wire.requests();
        assert!(!requests[0].streaming);
        assert_eq!(requests[0].request.url.as_str(), STREAM_URL);
    }
    assert!(fixture.state.httpx_response.is_none());
    let SyncStream {
        mut iterator,
        metadata,
    } = *stream;
    assert_eq!(
        iterator.next().expect("one event").expect("a good event"),
        SSE.as_bytes()
    );
    assert!(iterator.next().is_none());
    assert_eq!(
        metadata.hidden_params.get("model_id"),
        Some(&decision(r#""deployment-7""#))
    );
}

#[tokio::test]
async fn an_async_stream_returns_the_iterator_and_its_metadata_before_the_first_event() {
    let events = Events::new();
    let wire = Wire::streaming(
        "async",
        Reply::json(200, SSE).with_header("x-request-id", "r1"),
        &events,
    );
    let source = Source::new(wire.clone());
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
    let mut fixture = Fixture::new("agenerate_content");

    let outcome = fixture
        .async_call(events.clone(), &deps, &target)
        .await
        .expect("the stream starts");

    let AsyncGenerateContentOutcome::Stream(stream) = outcome else {
        panic!("a streaming call answers an iterator");
    };
    let AsyncStream {
        mut iterator,
        metadata,
    } = *stream;
    assert_eq!(metadata.model, MODEL);
    assert_eq!(
        metadata.hidden_params.get("model_id"),
        Some(&decision(r#""gemini-2.0-flash""#))
    );
    assert_eq!(
        metadata.hidden_params.get("api_base"),
        Some(&decision(&format!("\"{STREAM_URL}\"")))
    );
    assert_eq!(
        metadata.hidden_params.get("additional_headers"),
        Some(&decision(r#"{"llm_provider-x-request-id":"r1"}"#))
    );
    {
        let requests = wire.requests();
        assert!(requests[0].streaming);
    }
    assert!(config.auth_calls()[0].stream);
    assert!(fixture.state.httpx_response.is_none());
    assert_eq!(
        iterator
            .next_event()
            .await
            .expect("the first event")
            .expect("a good event"),
        SSE.as_bytes()
    );
    assert!(iterator.next_event().await.expect("the end").is_none());
}

#[tokio::test]
async fn an_async_stream_over_a_buffered_reply_still_yields_its_events() {
    let events = Events::new();
    let wire = Wire::replying("async", Reply::json(200, SSE), &events);
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
    let mut fixture = Fixture::new("agenerate_content");

    let outcome = fixture
        .async_call(events.clone(), &deps, &target)
        .await
        .expect("the stream starts");

    let AsyncGenerateContentOutcome::Stream(stream) = outcome else {
        panic!("a streaming call answers an iterator");
    };
    let AsyncStream { mut iterator, .. } = *stream;
    assert_eq!(
        iterator
            .next_event()
            .await
            .expect("the first event")
            .expect("a good event"),
        SSE.as_bytes()
    );
    assert!(iterator.next_event().await.expect("the end").is_none());
}

#[tokio::test]
async fn stream_metadata_falls_back_to_the_request_model() {
    let events = Events::new();
    let wire = Wire::streaming("sync", Reply::json(200, SSE), &events);
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
    let params = kwargs(r#"{"model_info":{"id":7}}"#);
    let target = call(&contents, &generation, &params, true, None);
    let mut fixture = Fixture::new("generate_content");

    let outcome = fixture
        .sync_call(&deps, &target, &events)
        .await
        .expect("the stream starts");

    let GenerateContentOutcome::Stream(stream) = outcome else {
        panic!("a streaming call answers an iterator");
    };
    assert_eq!(
        stream.metadata.hidden_params.get("model_id"),
        Some(&decision(r#""gemini-2.0-flash""#))
    );
}

#[tokio::test]
async fn a_supplied_client_of_the_callers_mode_is_used() {
    let events = Events::new();
    let supplied = Wire::replying("supplied", Reply::json(200, REPLY), &events);
    let created = Wire::replying("created", Reply::json(200, REPLY), &events);
    let source = Source::new(created);
    let config = TestConfig::new("https://gemini.test");
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &supplied,
        }),
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");

    fixture
        .sync_call(&deps, &target, &events)
        .await
        .expect("the call succeeds");

    assert_eq!(
        events.recorded(),
        vec!["log".to_owned(), "supplied".to_owned()]
    );
    assert!(source.calls().is_empty());
}

#[tokio::test]
async fn a_supplied_client_of_the_other_mode_is_ignored() {
    let events = Events::new();
    let supplied = Wire::replying("supplied", Reply::json(200, REPLY), &events);
    let created = Wire::replying("created", Reply::json(200, REPLY), &events);
    let source = Source::new(created);
    let config = TestConfig::new("https://gemini.test");
    let deps = GenerateContentDeps {
        config: &config,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: ClientKind::Sync,
            transport: &supplied,
        }),
        stream: StreamSettings::default(),
    };
    let contents = json(CONTENTS);
    let generation = object(CONFIG);
    let params = kwargs("{}");
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("agenerate_content");

    fixture
        .async_call(events.clone(), &deps, &target)
        .await
        .expect("the call succeeds");

    assert_eq!(
        events.recorded(),
        vec!["log".to_owned(), "created".to_owned()]
    );
    assert_eq!(source.calls(), vec![None]);
}

#[tokio::test]
async fn the_client_source_receives_ssl_verify() {
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
    let params = kwargs(r#"{"ssl_verify":false}"#);
    let target = call(&contents, &generation, &params, false, None);
    let mut fixture = Fixture::new("generate_content");

    fixture
        .sync_call(&deps, &target, &events)
        .await
        .expect("the call succeeds");

    assert_eq!(source.calls(), vec![Some(json("false"))]);
}

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
