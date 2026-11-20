#![cfg(test)]

mod support;

use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::speech_provider_contract::TextToSpeechRequestData;
use betterlite_openai::openai_base_config::Object;
use http::Method;
use http::header::HeaderValue;
use indexmap::IndexMap;
use rstest::rstest;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use self::support::{
    Fixture, Harness, IMPATIENT, ProxiedSource, RecordingConfig, Reply, URL, Wire, body_bytes,
    body_json, bytes_content_type, captured, data, header, headers, json, object, params, run,
};
use super::error::SpeechError;
use super::facts::{MISSING_BODY, format_timeout};
use super::{Mode, SpeechCall, SpeechDeps, SpeechParams, SuppliedClient};
use crate::api::speech_provider_contract::SpeechHeaders;
use crate::api::speech_provider_contract::error::SpeechContractError;

fn provider_headers() -> IndexMap<String, Json> {
    IndexMap::from([("x".to_owned(), Json::Text("provider".to_owned()))])
}

fn headers_json(pairs: &[(&str, &str)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), Json::Text((*value).to_owned())))
        .collect()
}

fn simple_call<'call>(
    params: &'call SpeechParams,
    optional: &'call Object,
    litellm: &'call Object,
) -> SpeechCall<'call> {
    SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: optional,
        litellm_params: litellm,
        extra_headers: None,
        params,
        timeout: None,
    }
}

#[tokio::test]
async fn speech_http_body_and_headers() {
    let mut harness = Harness::new(Reply::bytes(200, &[0x00, 0x01, 0x02]));
    harness.config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(
            Some(r#"{"input":"hello"}"#),
            Some("ignored"),
            Some(headers(&[("x", "provider")])),
        ),
        headers(&[("x", "validated")]),
    );
    harness.declared = params(Some("key"), Some("http://declared.test"), None);
    harness.caller = headers(&[("x", "caller")]);
    harness.timeout = Some(Duration::from_secs(5));
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the speech call succeeds");
    let validations = harness.config.validations();
    assert_eq!(validations[0].api_key.as_deref(), Some("key"));
    assert_eq!(
        validations[0].api_base.as_deref(),
        Some("http://declared.test")
    );
    assert_eq!(validations[0].headers, headers(&[("x", "caller")]));
    assert_eq!(
        fixture.logged("headers"),
        Some(&Json::Object(provider_headers()))
    );
    let requests = harness.wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].url.as_str(), URL);
    assert_eq!(requests[0].timeout, Some(Duration::from_secs(5)));
    assert_eq!(header(&requests[0], "x").as_deref(), Some("provider"));
    assert_eq!(header(&requests[0], "content-type"), None);
    assert_eq!(bytes_content_type(&requests[0]), None);
    assert_eq!(body_json(&requests[0]), json(r#"{"input":"hello"}"#));
}

#[tokio::test]
async fn speech_http_transform_and_pre_call() {
    let mut harness = Harness::new(Reply::bytes(200, b"audio"));
    harness.config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(
            Some(r#"{"input":"hello"}"#),
            Some("ignored"),
            Some(headers(&[("x", "provider")])),
        ),
        headers(&[("x", "validated")]),
    );
    harness.optional = object(r#"{"speed":1}"#);
    harness.litellm = object(r#"{"custom":true}"#);
    harness.caller = headers(&[("x", "caller")]);
    harness.voice = Some("alloy");
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the speech call succeeds");
    let observed = harness.config.observed();
    assert_eq!(observed[0].model, "tts-1");
    assert_eq!(observed[0].input, "hello");
    assert_eq!(observed[0].voice.as_deref(), Some("alloy"));
    assert_eq!(observed[0].optional_params, harness.optional);
    assert_eq!(observed[0].litellm_params, harness.litellm);
    assert_eq!(observed[0].headers, headers(&[("x", "caller")]));
    assert_eq!(fixture.input(), Some(&Json::Text("hello".to_owned())));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(URL.to_owned()))
    );
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(
            r#"{"dict_body":{"input":"hello"},"ssml_body":"ignored","headers":{"x":"provider"}}"#
        ))
    );
    assert_eq!(fixture.logged("api_key"), None);
}

#[tokio::test]
async fn speech_http_response_content() {
    let reply = Reply::bytes(200, &[0x00, 0x01, 0x02])
        .with_header("content-type", "audio/wav")
        .with_header("x-request-id", "speech_1")
        .with_header("x-ratelimit-remaining-tokens", "9");
    let harness = Harness::new(reply);
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    let response = run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the speech call succeeds");
    assert_eq!(response.content.status, 200);
    assert_eq!(response.content.body, vec![0x00, 0x01, 0x02]);
    let content_headers = &response.content.headers;
    assert_eq!(
        content_headers.get("x-request-id").map(String::as_str),
        Some("speech_1")
    );
    assert_eq!(response.hidden.headers, response.content.headers);
    let hidden = &response.hidden.additional_headers;
    assert_eq!(
        hidden.get("x-ratelimit-remaining-tokens"),
        Some(&Json::Text("9".to_owned()))
    );
    assert_eq!(
        hidden.get("llm_provider-x-ratelimit-remaining-tokens"),
        Some(&Json::Text("9".to_owned()))
    );
    assert_eq!(
        hidden.get("llm_provider-x-request-id"),
        Some(&Json::Text("speech_1".to_owned()))
    );
    assert_eq!(harness.config.responses(), 1);
}

#[tokio::test]
async fn speech_http_empty_dict_body() {
    let mut harness = Harness::new(Reply::bytes(200, b"audio"));
    harness.config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(Some("{}"), Some("ignored"), None),
        headers(&[("x", "validated")]),
    );
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the empty body is still a JSON body");
    let requests = harness.wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(bytes_content_type(&requests[0]), None);
    assert_eq!(body_json(&requests[0]), json("{}"));
}

#[tokio::test]
async fn speech_http_sync_and_async_send_the_same_http() {
    let mut harness = Harness::new(Reply::bytes(200, b"audio"));
    harness.caller = headers(&[("x", "caller")]);
    harness.voice = Some("alloy");
    harness.timeout = Some(Duration::from_secs(5));
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the sync call succeeds");
    run(&mut fixture, &deps, Mode::Asynchronous, &call)
        .await
        .expect("the async call succeeds");
    let captures = captured(&harness.wire);
    assert_eq!(captures.len(), 2);
    assert_eq!(captures[0], captures[1]);
    assert_eq!(captures[0].method, "POST");
    assert_eq!(captures[0].url, URL);
    assert_eq!(
        captures[0].headers,
        vec![("x".to_owned(), "caller".to_owned())]
    );
    assert_eq!(captures[0].body.as_deref(), Some(br#"{"input":"hello"}"#));
    let requests = harness.wire.requests();
    assert_eq!(requests[0].timeout, Some(Duration::from_secs(5)));
    assert_eq!(requests[1].timeout, Some(Duration::from_secs(5)));
}

#[tokio::test]
async fn speech_http_ssml() {
    let mut harness = Harness::new(Reply::bytes(200, b"audio"));
    harness.config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(None, Some("<speak>hello</speak>"), None),
        headers(&[("x", "validated"), ("y", "validated")]),
    );
    harness.caller = headers(&[("x", "caller")]);
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the ssml call succeeds");
    let requests = harness.wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        body_bytes(&requests[0]),
        Some(b"<speak>hello</speak>".as_slice())
    );
    assert_eq!(
        bytes_content_type(&requests[0]),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(header(&requests[0], "x").as_deref(), Some("caller"));
    assert_eq!(header(&requests[0], "y").as_deref(), Some("validated"));
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(r#"{"ssml_body":"<speak>hello</speak>"}"#))
    );
    assert_eq!(
        fixture.logged("headers"),
        Some(&Json::Object(headers_json(&[
            ("x", "caller"),
            ("y", "validated")
        ])))
    );
}

#[tokio::test]
async fn speech_http_request_data_headers_win() {
    let mut harness = Harness::new(Reply::bytes(200, b"audio"));
    harness.config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(
            Some(r#"{"input":"hello"}"#),
            None,
            Some(headers(&[("authorization", "Bearer provider")])),
        ),
        headers(&[("authorization", "Bearer validated")]),
    );
    harness.caller = headers(&[("authorization", "Bearer caller")]);
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the speech call succeeds");
    let requests = harness.wire.requests();
    assert_eq!(
        header(&requests[0], "authorization").as_deref(),
        Some("Bearer provider")
    );
}

#[rstest]
#[case::sync_reuses_supplied(Mode::Sync, Mode::Sync, true)]
#[case::async_reuses_supplied(Mode::Asynchronous, Mode::Asynchronous, true)]
#[case::sync_replaces_async(Mode::Sync, Mode::Asynchronous, false)]
#[case::async_replaces_sync(Mode::Asynchronous, Mode::Sync, false)]
#[tokio::test]
async fn speech_http_client_selection(
    #[case] handler: Mode,
    #[case] kind: Mode,
    #[case] reused: bool,
) {
    let harness = Harness::new(Reply::bytes(200, b"audio"));
    let supplied = Wire::replying(Reply::bytes(200, b"audio"));
    let call = harness.call();
    let deps = SpeechDeps {
        config: &harness.config,
        source: &harness.source,
        supplied: Some(SuppliedClient {
            kind,
            transport: &supplied,
        }),
    };
    let mut fixture = Fixture::new();
    run(&mut fixture, &deps, handler, &call)
        .await
        .expect("the handler sends");
    assert_eq!(supplied.requests().len(), usize::from(reused));
    assert_eq!(harness.wire.requests().len(), usize::from(!reused));
    assert_eq!(harness.source.calls().len(), usize::from(!reused));
}

#[tokio::test]
async fn speech_http_ssl_verify_reaches_the_pool() {
    let mut harness = Harness::new(Reply::bytes(200, b"audio"));
    harness.declared = params(None, Some(URL), Some(false));
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    run(&mut fixture, &deps, Mode::Asynchronous, &call)
        .await
        .expect("the async pooled client sends");
    assert_eq!(harness.source.calls(), vec![Some(false)]);
}

#[tokio::test]
async fn speech_http_missing_body() {
    let mut harness = Harness::new(Reply::bytes(200, b"audio"));
    harness.config = RecordingConfig::new(
        Ok(URL.to_owned()),
        TextToSpeechRequestData::default(),
        SpeechHeaders::new(),
    );
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    let Err(SpeechError::Provider(error)) = run(&mut fixture, &deps, Mode::Sync, &call).await
    else {
        panic!("a missing body is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(error.message, MISSING_BODY);
    assert!(error.headers.is_empty());
    assert!(harness.wire.requests().is_empty());
    assert_eq!(harness.config.responses(), 0);
    assert_eq!(
        harness.config.errors(),
        vec![(MISSING_BODY.to_owned(), 500, SpeechHeaders::new())]
    );
    assert_eq!(fixture.input(), Some(&Json::Text("hello".to_owned())));
}

#[tokio::test]
async fn speech_http_status_error() {
    let reply = Reply::bytes(429, b"slow")
        .with_header("content-type", "text/plain")
        .with_header("content-encoding", "gzip")
        .with_header("retry-after", "3");
    let harness = Harness::new(reply);
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    let Err(SpeechError::Provider(error)) = run(&mut fixture, &deps, Mode::Sync, &call).await
    else {
        panic!("a 429 is a provider error");
    };
    assert_eq!(error.status_code, 429);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "slow");
    assert_eq!(
        error.headers.get("retry-after").map(String::as_str),
        Some("3")
    );
    assert_eq!(
        error.headers.get("content-type").map(String::as_str),
        Some("text/plain")
    );
    assert_eq!(
        error.headers.get("content-length").map(String::as_str),
        Some("4")
    );
    assert_eq!(error.headers.get("content-encoding"), None);
    assert_eq!(harness.config.responses(), 0);
    assert_eq!(
        harness.config.errors(),
        vec![("slow".to_owned(), 429, error.headers)]
    );
}

#[tokio::test]
async fn speech_http_status_error_without_body() {
    let reply = Reply::bytes(429, b"").with_header("retry-after", "3");
    let harness = Harness::new(reply);
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    let Err(SpeechError::Provider(error)) = run(&mut fixture, &deps, Mode::Sync, &call).await
    else {
        panic!("a 429 is a provider error");
    };
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "");
    assert_eq!(error.headers, headers(&[("retry-after", "3")]));
}

#[tokio::test]
async fn speech_http_status_error_binary_text() {
    let mut reply = Reply::bytes(429, &[0xff, 0xfe]);
    reply.headers.insert(
        "x-binary",
        HeaderValue::from_bytes(&[0xff]).expect("an opaque header value"),
    );
    let harness = Harness::new(reply);
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    let Err(SpeechError::Provider(error)) = run(&mut fixture, &deps, Mode::Sync, &call).await
    else {
        panic!("a 429 with binary text is a provider error");
    };
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "2 bytes that are not UTF-8");
    assert_eq!(
        error.headers.get("x-binary").map(String::as_str),
        Some("1 bytes that are not UTF-8")
    );
    assert_eq!(
        error.headers.get("content-length").map(String::as_str),
        Some("2")
    );
}

#[tokio::test]
async fn speech_http_transport_failure() {
    let harness = Harness::from_wire(Wire::failing(TransportError::Closed));
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    let Err(SpeechError::Provider(error)) = run(&mut fixture, &deps, Mode::Sync, &call).await
    else {
        panic!("a transport failure is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(error.message, TransportError::Closed.to_string());
    assert!(error.headers.is_empty());
    assert_eq!(harness.config.responses(), 0);
    assert_eq!(
        harness.config.errors(),
        vec![(
            TransportError::Closed.to_string(),
            500,
            SpeechHeaders::new()
        )]
    );
}

async fn stalled() -> (JoinSet<()>, ProxiedSource, String) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener binds");
    let address = listener.local_addr().expect("the listener has an address");
    let mut server = JoinSet::new();
    server.spawn(stall(listener));
    (
        server,
        ProxiedSource::new(IMPATIENT),
        format!("http://{address}/speech"),
    )
}

async fn stall(listener: TcpListener) {
    let (stream, _) = listener.accept().await.expect("the connection arrives");
    sleep(Duration::from_millis(500)).await;
    drop(stream);
}

#[tokio::test]
async fn speech_http_timeout_sync() {
    let (mut server, source, url) = stalled().await;
    let config = RecordingConfig::new(
        Ok(url),
        data(Some(r#"{"input":"hello"}"#), None, None),
        SpeechHeaders::new(),
    );
    let deps = SpeechDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let declared = params(None, None, None);
    let optional = Object::new();
    let litellm = Object::new();
    let call = simple_call(&declared, &optional, &litellm);
    let mut fixture = Fixture::new();
    let Err(SpeechError::Provider(error)) = run(&mut fixture, &deps, Mode::Sync, &call).await
    else {
        panic!("a timeout is a provider error");
    };
    assert_eq!(error.status_code, 408);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(
        error.message,
        "litellm.Timeout: Connection timed out after None seconds."
    );
    assert!(error.headers.is_empty());
    server.shutdown().await;
}

#[tokio::test]
async fn speech_http_timeout_async() {
    let (mut server, source, url) = stalled().await;
    let config = RecordingConfig::new(
        Ok(url),
        data(Some(r#"{"input":"hello"}"#), None, None),
        SpeechHeaders::new(),
    );
    let deps = SpeechDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let declared = params(None, None, None);
    let optional = Object::new();
    let litellm = Object::new();
    let call = simple_call(&declared, &optional, &litellm);
    let mut fixture = Fixture::new();
    let Err(SpeechError::Provider(error)) =
        run(&mut fixture, &deps, Mode::Asynchronous, &call).await
    else {
        panic!("a timeout is a provider error");
    };
    assert_eq!(error.status_code, 408);
    assert!(!error.status_code_is_synthesized);
    assert!(
        error
            .message
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken=")
    );
    assert!(error.message.ends_with(" seconds"));
    assert!(!error.message.contains("time taken=0.000"));
    assert!(error.headers.is_empty());
    server.shutdown().await;
}

#[test]
fn speech_http_timeout_message_names_the_passed_timeout() {
    assert_eq!(format_timeout(None), "None");
    assert_eq!(format_timeout(Some(Duration::from_secs(5))), "5.0");
    assert_eq!(format_timeout(Some(Duration::from_millis(100))), "0.1");
    assert_eq!(format_timeout(Some(Duration::from_millis(1500))), "1.5");
}

#[tokio::test]
async fn speech_http_complete_url_error() {
    let mut harness = Harness::new(Reply::bytes(200, b"audio"));
    harness.config = RecordingConfig::new(
        Err(SpeechContractError::ApiBaseRequired),
        data(Some(r#"{"input":"hello"}"#), None, None),
        SpeechHeaders::new(),
    );
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    let result = run(&mut fixture, &deps, Mode::Sync, &call).await;
    assert!(matches!(result, Err(SpeechError::CompleteUrl(_))));
    assert!(harness.wire.requests().is_empty());
    assert_eq!(fixture.input(), None);
}

#[tokio::test]
async fn speech_http_invalid_url() {
    let mut harness = Harness::new(Reply::bytes(200, b"audio"));
    harness.config = RecordingConfig::new(
        Ok("provider.test/no-scheme".to_owned()),
        data(Some(r#"{"input":"hello"}"#), None, None),
        SpeechHeaders::new(),
    );
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    let result = run(&mut fixture, &deps, Mode::Sync, &call).await;
    assert!(matches!(result, Err(SpeechError::RequestUrl { .. })));
    assert!(harness.wire.requests().is_empty());
}

#[rstest]
#[case::name("x y", "ok")]
#[case::value("x-ok", "line\nbreak")]
#[tokio::test]
async fn speech_http_invalid_header(#[case] name: &str, #[case] value: &str) {
    let mut harness = Harness::new(Reply::bytes(200, b"audio"));
    harness.caller = headers(&[(name, value)]);
    let deps = harness.deps();
    let call = harness.call();
    let mut fixture = Fixture::new();
    let result = run(&mut fixture, &deps, Mode::Sync, &call).await;
    if name.contains(' ') {
        assert!(matches!(result, Err(SpeechError::HeaderName { .. })));
    } else {
        assert!(matches!(result, Err(SpeechError::HeaderValue { .. })));
    }
    assert!(fixture.input().is_some());
    assert!(harness.wire.requests().is_empty());
}
