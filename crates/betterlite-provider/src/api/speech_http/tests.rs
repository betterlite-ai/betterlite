#![cfg(test)]

mod support;

use std::time::Duration;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::speech_provider_contract::TextToSpeechRequestData;
use betterlite_openai::openai_base_config::Object;
use http::Method;
use indexmap::IndexMap;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use self::support::{
    Capture, Fixture, IMPATIENT, ProxiedSource, RecordingConfig, Reply, Source, Wire, body_bytes,
    body_json, bytes_content_type, captured, header, headers, params, run,
};
use super::error::SpeechError;
use super::facts::{MISSING_BODY, format_timeout};
use super::{Mode, SpeechCall, SpeechDeps, SpeechParams, SuppliedClient};
use crate::api::speech_provider_contract::{SpeechContractError, SpeechHeaders};

const URL: &str = "http://provider.test/v1/audio/speech";

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the object fixture parses")
}

fn data(
    dict: Option<&str>,
    ssml: Option<&str>,
    provider: Option<SpeechHeaders>,
) -> TextToSpeechRequestData {
    TextToSpeechRequestData {
        dict_body: dict.map(object),
        ssml_body: ssml.map(ToOwned::to_owned),
        headers: provider,
    }
}

fn provider_headers() -> IndexMap<String, Json> {
    IndexMap::from([("x".to_owned(), Json::Text("provider".to_owned()))])
}

#[tokio::test]
async fn speech_http_body_and_headers() {
    let reply = Reply::bytes(200, &[0x00, 0x01, 0x02])
        .with_header("content-type", "audio/wav")
        .with_header("x-request-id", "speech_1")
        .with_header("x-ratelimit-remaining-tokens", "9");
    let wire = Wire::replying(reply);
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(
            Some(r#"{"input":"hello"}"#),
            Some("ignored"),
            Some(headers(&[("x", "provider")])),
        ),
        headers(&[("x", "validated")]),
    );
    let deps = SpeechDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let declared = params(Some("key"), Some("http://declared.test"), None);
    let optional = object(r#"{"speed":1}"#);
    let litellm = object(r#"{"custom":true}"#);
    let caller = headers(&[("x", "caller")]);
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: Some("alloy"),
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: Some(&caller),
        params: &declared,
        timeout: Some(Duration::from_secs(5)),
    };
    let mut fixture = Fixture::new();
    let response = run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the speech call succeeds");

    let validations = config.validations();
    assert_eq!(validations[0].api_key.as_deref(), Some("key"));
    assert_eq!(
        validations[0].api_base.as_deref(),
        Some("http://declared.test")
    );
    assert_eq!(validations[0].headers, headers(&[("x", "caller")]));
    let observed = config.observed();
    assert_eq!(observed[0].model, "tts-1");
    assert_eq!(observed[0].input, "hello");
    assert_eq!(observed[0].voice.as_deref(), Some("alloy"));
    assert_eq!(observed[0].optional_params, optional);
    assert_eq!(observed[0].litellm_params, litellm);
    assert_eq!(observed[0].headers, headers(&[("x", "caller")]));

    {
        let requests = wire.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, Method::POST);
        assert_eq!(requests[0].url.as_str(), URL);
        assert_eq!(header(&requests[0], "x").as_deref(), Some("provider"));
        assert_eq!(header(&requests[0], "content-type"), None);
        assert_eq!(bytes_content_type(&requests[0]), None);
        assert_eq!(body_json(&requests[0]), json(r#"{"input":"hello"}"#));
    }

    assert_eq!(response.content.status, 200);
    assert_eq!(response.content.body, vec![0x00, 0x01, 0x02]);
    assert_eq!(
        response
            .content
            .headers
            .get("x-request-id")
            .map(String::as_str),
        Some("speech_1")
    );
    assert_eq!(response.hidden.headers, response.content.headers);
    assert_eq!(
        response
            .hidden
            .additional_headers
            .get("x-ratelimit-remaining-tokens"),
        Some(&Json::Text("9".to_owned()))
    );
    assert_eq!(
        response
            .hidden
            .additional_headers
            .get("llm_provider-x-ratelimit-remaining-tokens"),
        Some(&Json::Text("9".to_owned()))
    );
    assert_eq!(
        response
            .hidden
            .additional_headers
            .get("llm_provider-x-request-id"),
        Some(&Json::Text("speech_1".to_owned()))
    );
    assert_eq!(config.responses(), 1);

    assert_eq!(fixture.input(), Some(&Json::Text("hello".to_owned())));
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(URL.to_owned()))
    );
    assert_eq!(
        fixture.logged("headers"),
        Some(&Json::Object(provider_headers()))
    );
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(
            r#"{"dict_body":{"input":"hello"},"ssml_body":"ignored","headers":{"x":"provider"}}"#
        ))
    );
}

#[tokio::test]
async fn speech_http_empty_dict_body() {
    let wire = Wire::replying(Reply::bytes(200, b"audio"));
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(Some("{}"), Some("ignored"), None),
        headers(&[("x", "validated")]),
    );
    let deps = SpeechDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let declared = params(None, Some(URL), None);
    let optional = Object::new();
    let litellm = Object::new();
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: None,
        params: &declared,
        timeout: None,
    };
    let mut fixture = Fixture::new();
    let _response = run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the empty body is still a JSON body");
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(bytes_content_type(&requests[0]), None);
    assert_eq!(body_json(&requests[0]), json("{}"));
}

#[tokio::test]
async fn speech_http_sync_and_async_send_the_same_http() {
    let declared = params(None, Some(URL), None);
    let optional = Object::new();
    let litellm = Object::new();
    let caller: SpeechHeaders = headers(&[("x", "caller")]);
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: Some("alloy"),
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: Some(&caller),
        params: &declared,
        timeout: None,
    };
    let sync_wire = Wire::replying(Reply::bytes(200, b"audio"));
    let sync_source = Source::new(sync_wire.clone());
    let sync_config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(Some(r#"{"input":"hello"}"#), None, None),
        headers(&[("x", "validated")]),
    );
    let sync_deps = SpeechDeps {
        config: &sync_config,
        source: &sync_source,
        supplied: None,
    };
    let mut fixture = Fixture::new();
    run(&mut fixture, &sync_deps, Mode::Sync, &call)
        .await
        .expect("the sync call succeeds");

    let async_wire = Wire::replying(Reply::bytes(200, b"audio"));
    let async_source = Source::new(async_wire.clone());
    let async_config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(Some(r#"{"input":"hello"}"#), None, None),
        headers(&[("x", "validated")]),
    );
    let async_deps = SpeechDeps {
        config: &async_config,
        source: &async_source,
        supplied: None,
    };
    run(&mut fixture, &async_deps, Mode::Asynchronous, &call)
        .await
        .expect("the async call succeeds");

    let expected = vec![Capture {
        method: "POST".to_owned(),
        url: URL.to_owned(),
        headers: vec![("x".to_owned(), "provider".to_owned())],
        body: Some(br#"{"input":"hello"}"#.to_vec()),
    }];
    assert_eq!(captured(&sync_wire), expected);
    assert_eq!(captured(&async_wire), expected);
}

#[tokio::test]
async fn speech_http_ssml() {
    let wire = Wire::replying(Reply::bytes(200, b"audio"));
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(None, Some("<speak>hello</speak>"), None),
        headers(&[("x", "validated")]),
    );
    let deps = SpeechDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let declared = params(None, Some(URL), None);
    let optional = Object::new();
    let litellm = Object::new();
    let caller = headers(&[("x", "caller")]);
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: Some(&caller),
        params: &declared,
        timeout: None,
    };
    let mut fixture = Fixture::new();
    let _response = run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the ssml call succeeds");
    let observed = config.observed();
    assert_eq!(observed[0].headers, headers(&[("x", "caller")]));
    {
        let requests = wire.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            body_bytes(&requests[0]).as_deref(),
            Some(b"<speak>hello</speak>".as_slice())
        );
        assert_eq!(
            bytes_content_type(&requests[0]),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(header(&requests[0], "x").as_deref(), Some("caller"));
    }
    assert_eq!(
        fixture.logged("complete_input_dict"),
        Some(&json(r#"{"ssml_body":"<speak>hello</speak>"}"#))
    );
    assert_eq!(
        fixture.logged("headers"),
        Some(&Json::Object(headers_json(&[("x", "caller")])))
    );
}

#[tokio::test]
async fn speech_http_request_data_headers_win() {
    let wire = Wire::replying(Reply::bytes(200, b"audio"));
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(
            Some(r#"{"input":"hello"}"#),
            None,
            Some(headers(&[("authorization", "Bearer provider")])),
        ),
        headers(&[("authorization", "Bearer validated")]),
    );
    let deps = SpeechDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let declared = params(None, Some(URL), None);
    let optional = Object::new();
    let litellm = Object::new();
    let caller = headers(&[("authorization", "Bearer caller")]);
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: Some(&caller),
        params: &declared,
        timeout: None,
    };
    let mut fixture = Fixture::new();
    let _response = run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the speech call succeeds");
    let requests = wire.requests();
    assert_eq!(
        header(&requests[0], "authorization").as_deref(),
        Some("Bearer provider")
    );
}

#[tokio::test]
async fn speech_http_client_selection() {
    let declared = params(None, Some(URL), None);
    let optional = Object::new();
    let litellm = Object::new();
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: None,
        params: &declared,
        timeout: None,
    };
    let config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(Some(r#"{"input":"hello"}"#), None, None),
        headers(&[("x", "validated")]),
    );

    let supplied_wire = Wire::replying(Reply::bytes(200, b"audio"));
    let source_wire = Wire::replying(Reply::bytes(200, b"audio"));
    let source = Source::new(source_wire.clone());
    let deps = SpeechDeps {
        config: &config,
        source: &source,
        supplied: Some(SuppliedClient {
            kind: Mode::Sync,
            transport: &supplied_wire,
        }),
    };
    let mut fixture = Fixture::new();
    run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the supplied client sends");
    assert_eq!(supplied_wire.requests().len(), 1);
    assert!(source.calls().is_empty());

    let wrong_wire = Wire::replying(Reply::bytes(200, b"audio"));
    let fallback_wire = Wire::replying(Reply::bytes(200, b"audio"));
    let fallback = Source::new(fallback_wire.clone());
    let deps = SpeechDeps {
        config: &config,
        source: &fallback,
        supplied: Some(SuppliedClient {
            kind: Mode::Asynchronous,
            transport: &wrong_wire,
        }),
    };
    run(&mut fixture, &deps, Mode::Sync, &call)
        .await
        .expect("the wrong-kind client falls back to the pool");
    assert!(wrong_wire.requests().is_empty());
    assert_eq!(fallback_wire.requests().len(), 1);
    assert_eq!(fallback.calls(), vec![None]);

    let verified = params(None, Some(URL), Some(false));
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: None,
        params: &verified,
        timeout: None,
    };
    let insecure_wire = Wire::replying(Reply::bytes(200, b"audio"));
    let insecure = Source::new(insecure_wire);
    let deps = SpeechDeps {
        config: &config,
        source: &insecure,
        supplied: None,
    };
    run(&mut fixture, &deps, Mode::Asynchronous, &call)
        .await
        .expect("the async pooled client sends");
    assert_eq!(insecure.calls(), vec![Some(false)]);
}

#[tokio::test]
async fn speech_http_missing_body() {
    let wire = Wire::replying(Reply::bytes(200, b"audio"));
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new(
        Ok(URL.to_owned()),
        TextToSpeechRequestData::default(),
        headers(&[("x", "validated")]),
    );
    let deps = SpeechDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let declared = params(None, Some(URL), None);
    let optional = Object::new();
    let litellm = Object::new();
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: None,
        params: &declared,
        timeout: None,
    };
    let mut fixture = Fixture::new();
    let Err(SpeechError::Provider(error)) = run(&mut fixture, &deps, Mode::Sync, &call).await
    else {
        panic!("a missing body is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(error.message, MISSING_BODY);
    assert!(error.headers.is_empty());
    assert!(wire.requests().is_empty());
    assert_eq!(config.responses(), 0);
    assert_eq!(
        config.errors(),
        vec![(MISSING_BODY.to_owned(), 500, SpeechHeaders::new())]
    );
    assert_eq!(fixture.input(), Some(&Json::Text("hello".to_owned())));
}

#[tokio::test]
async fn speech_http_status_error() {
    let wire = Wire::replying(
        Reply::bytes(429, b"slow")
            .with_header("content-type", "text/plain")
            .with_header("retry-after", "3"),
    );
    let source = Source::new(wire);
    let config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(Some(r#"{"input":"hello"}"#), None, None),
        headers(&[("x", "validated")]),
    );
    let deps = SpeechDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let declared = params(None, Some(URL), None);
    let optional = Object::new();
    let litellm = Object::new();
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: None,
        params: &declared,
        timeout: None,
    };
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
    assert_eq!(config.responses(), 0);
    assert_eq!(
        config.errors(),
        vec![("slow".to_owned(), 429, error.headers.clone())]
    );
}

#[tokio::test]
async fn speech_http_status_error_without_body() {
    let wire = Wire::replying(Reply::bytes(429, b"").with_header("retry-after", "3"));
    let source = Source::new(wire);
    let config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(Some(r#"{"input":"hello"}"#), None, None),
        headers(&[("x", "validated")]),
    );
    let deps = SpeechDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let declared = params(None, Some(URL), None);
    let optional = Object::new();
    let litellm = Object::new();
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: None,
        params: &declared,
        timeout: None,
    };
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
async fn speech_http_transport_failure() {
    let wire = Wire::failing(TransportError::Closed);
    let source = Source::new(wire);
    let config = RecordingConfig::new(
        Ok(URL.to_owned()),
        data(Some(r#"{"input":"hello"}"#), None, None),
        headers(&[("x", "validated")]),
    );
    let deps = SpeechDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let declared = params(None, Some(URL), None);
    let optional = Object::new();
    let litellm = Object::new();
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: None,
        params: &declared,
        timeout: None,
    };
    let mut fixture = Fixture::new();
    let Err(SpeechError::Provider(error)) = run(&mut fixture, &deps, Mode::Sync, &call).await
    else {
        panic!("a transport failure is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(error.message, TransportError::Closed.to_string());
    assert!(error.headers.is_empty());
    assert_eq!(config.responses(), 0);
    assert_eq!(
        config.errors(),
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
    tokio::time::sleep(Duration::from_millis(500)).await;
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
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: None,
        params: &declared,
        timeout: None,
    };
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
    let call = SpeechCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: &litellm,
        extra_headers: None,
        params: &declared,
        timeout: None,
    };
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
async fn speech_http_complete_url_error() {
    let wire = Wire::replying(Reply::bytes(200, b"audio"));
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new(
        Err(SpeechContractError::ApiBaseRequired),
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
    let result = run(&mut fixture, &deps, Mode::Sync, &call).await;
    assert!(matches!(result, Err(SpeechError::CompleteUrl(_))));
    assert!(wire.requests().is_empty());
    assert_eq!(fixture.input(), None);
}

#[tokio::test]
async fn speech_http_invalid_url() {
    let wire = Wire::replying(Reply::bytes(200, b"audio"));
    let source = Source::new(wire.clone());
    let config = RecordingConfig::new(
        Ok("provider.test/no-scheme".to_owned()),
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
    let result = run(&mut fixture, &deps, Mode::Sync, &call).await;
    assert!(matches!(result, Err(SpeechError::RequestUrl { .. })));
    assert!(wire.requests().is_empty());
}

#[tokio::test]
async fn speech_http_invalid_header() {
    let wire = Wire::replying(Reply::bytes(200, b"audio"));
    let source = Source::new(wire.clone());
    for (name, value) in [("x y", "ok"), ("x-ok", "line\nbreak")] {
        let config = RecordingConfig::new(
            Ok(URL.to_owned()),
            data(Some(r#"{"input":"hello"}"#), None, None),
            headers(&[(name, value)]),
        );
        let deps = SpeechDeps {
            config: &config,
            source: &source,
            supplied: None,
        };
        let declared = params(None, Some(URL), None);
        let optional = Object::new();
        let litellm = Object::new();
        let call = simple_call(&declared, &optional, &litellm);
        let mut fixture = Fixture::new();
        let result = run(&mut fixture, &deps, Mode::Sync, &call).await;
        if name.contains(' ') {
            assert!(matches!(result, Err(SpeechError::HeaderName { .. })));
        } else {
            assert!(matches!(result, Err(SpeechError::HeaderValue { .. })));
        }
        assert!(fixture.input().is_some());
    }
    assert!(wire.requests().is_empty());
}
