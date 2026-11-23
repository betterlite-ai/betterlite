use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::speech::binary::finish_audio_speech;
use crate::speech::dispatch::{
    DispatchCall, SpeechEnvironment, SpeechGlobals, dispatch_text_to_speech,
    supported_openai_params,
};
use crate::speech::error::AzureSpeechError;
use crate::speech::probe::{ProbeCall, ProbeMode, finish_probe, probe_request};

use super::support::{
    self, API_BASE, API_KEY, API_VERSION, HandlerRecord, Inputs, Recorder, RecordingHandler, json,
};

#[rstest]
#[case::openai_audio_speech_path(0)]
#[case::v1_audio_speech(1)]
#[case::native_speech_selection(2)]
#[case::binary_speech_body(3)]
#[case::azure_speech_supported_params(4)]
#[case::azure_rate_limit_probe(5)]
#[case::azure_probe_cloudflare(6)]
#[tokio::test]
async fn speech_cases(#[case] case: u8) {
    match case {
        0 => openai_audio_speech_path(),
        1 => v1_audio_speech(),
        2 => native_speech_selection().await,
        3 => binary_speech_body(),
        4 => azure_speech_supported_params(),
        5 => azure_rate_limit_probe(),
        6 => azure_probe_cloudflare(),
        _ => panic!("the speech case is known"),
    }
}

fn openai_audio_speech_path() {
    let mut inputs = Inputs::default();
    let mut logger = Recorder::default();
    let call = support::call("tts", "hello", "alloy", IndexMap::new());
    let request =
        support::start(&mut inputs, &call, &mut logger).expect("the speech request builds");
    assert_eq!(request.method, http::Method::POST);
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/deployments/tts/audio/speech?api-version=2024-06-01"
    );
    assert_eq!(support::header(&request, "api-key"), Some(API_KEY));
    assert_eq!(
        support::header(&request, "accept"),
        Some("application/octet-stream")
    );
    assert_eq!(request.deadline, Duration::from_secs(600));
    assert_eq!(
        support::body(&request),
        json(r#"{"model":"tts","input":"hello","voice":"alloy"}"#)
    );
    let pre = logger.pre.expect("the pre-call runs");
    assert_eq!(pre.input, "hello");
    assert_eq!(pre.api_base, "https://example.openai.azure.com/openai/");
    assert_eq!(pre.data, json(r#"{"model":"tts","voice":"alloy"}"#));
    assert_eq!(inputs.cache.entries.len(), 1);
    let content = finish_audio_speech(support::response(
        200,
        &[("content-type", "audio/mpeg")],
        b"ID3",
    ))
    .expect("the bytes stay");
    assert_eq!(content.status, 200);
    assert_eq!(content.body, b"ID3".to_vec());
    assert_eq!(
        content.headers.get("content-type").map(String::as_str),
        Some("audio/mpeg")
    );
    let failed = finish_audio_speech(support::response(400, &[], b"bad input"))
        .expect_err("a non-2xx reply is the status error");
    assert_eq!(failed.class(), "APIStatusError");
    assert_eq!(failed.status(), Some(400));
}

fn v1_audio_speech() {
    let mut inputs = Inputs::default();
    let mut logger = Recorder::default();
    let mut call = support::call("tts", "hello", "alloy", IndexMap::new());
    call.api_version = Some(String::from("v1"));
    let request = support::start(&mut inputs, &call, &mut logger).expect("the v1 request builds");
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/v1/audio/speech"
    );
    assert_eq!(
        support::header(&request, "authorization"),
        Some("Bearer sk-azure-fixture")
    );
    assert_eq!(support::header(&request, "api-key"), None);
    assert_eq!(
        support::body(&request),
        json(r#"{"model":"tts","input":"hello","voice":"alloy"}"#)
    );
}

async fn native_speech_selection() {
    let headers = BTreeMap::from([(String::from("x-call"), String::from("1"))]);
    let optional = support::pairs(&[("speed", json("1.5"))]);
    let globals = SpeechGlobals {
        api_base: Some(String::from("https://global.example")),
        api_key: Some(SecretString::from("sk-global-key")),
        azure_key: Some(SecretString::from("sk-azure-global")),
    };
    let environment = BTreeMap::from([
        (
            String::from("AZURE_API_BASE"),
            String::from("https://env.example"),
        ),
        (
            String::from("AZURE_OPENAI_API_KEY"),
            String::from("sk-openai-env"),
        ),
        (String::from("AZURE_API_KEY"), String::from("sk-azure-env")),
    ]);
    let speech_environment = SpeechEnvironment {
        globals: &globals,
        environment: &environment,
    };
    let voice = json(r#"{"name":"alloy"}"#);
    let call = DispatchCall {
        model: "tts-1",
        input: "hello",
        voice: Some(&voice),
        optional_params: &optional,
        litellm_params: IndexMap::new(),
        api_base: None,
        api_key: None,
        timeout: Some(Duration::from_secs(5)),
        extra_headers: Some(&headers),
        aspeech: true,
        environment: &speech_environment,
    };
    let mut handler = RecordingHandler::default();
    dispatch_text_to_speech(call, &mut handler)
        .await
        .expect("the handler answers");
    let request = first(&handler);
    assert_eq!(request.api_base.as_deref(), Some("https://global.example"));
    assert_eq!(request.api_key.as_deref(), Some("sk-global-key"));
    assert_eq!(request.voice.as_deref(), Some("alloy"));
    assert_eq!(request.provider, "azure");
    assert!(request.is_async);
    assert_eq!(request.model, "tts-1");
    assert_eq!(request.input, "hello");
    assert_eq!(request.timeout, Some(Duration::from_secs(5)));
    assert_eq!(request.extra_headers, Some(headers.clone()));
    assert_eq!(
        request.litellm_params.get("api_base"),
        Some(&Json::Text(String::from("https://global.example")))
    );
    assert_eq!(
        request.litellm_params.get("api_key"),
        Some(&Json::Text(String::from("sk-global-key")))
    );
    assert_eq!(request.optional_params, optional);
    resolve_chains(&globals, &environment).await;
    voice_rules(&globals, &environment, &optional).await;
}

// Every slot of the base and key chains resolves on its own, in the source order.
async fn resolve_chains(globals: &SpeechGlobals, environment: &BTreeMap<String, String>) {
    let optional = IndexMap::new();
    let speech_environment = SpeechEnvironment {
        globals,
        environment,
    };
    let params = support::pairs(&[
        ("api_base", json("\"https://params.example\"")),
        ("api_key", json("\"sk-params\"")),
    ]);
    let call = DispatchCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: params.clone(),
        api_base: Some(String::from("https://explicit.example")),
        api_key: Some(SecretString::from("sk-explicit")),
        timeout: None,
        extra_headers: None,
        aspeech: false,
        environment: &speech_environment,
    };
    let request = resolved(call).await;
    assert_eq!(
        request.api_base.as_deref(),
        Some("https://explicit.example")
    );
    assert_eq!(request.api_key.as_deref(), Some("sk-explicit"));
    assert!(!request.is_async);
    let call = DispatchCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: params,
        api_base: None,
        api_key: None,
        timeout: None,
        extra_headers: None,
        aspeech: false,
        environment: &speech_environment,
    };
    let request = resolved(call).await;
    assert_eq!(request.api_base.as_deref(), Some("https://params.example"));
    assert_eq!(request.api_key.as_deref(), Some("sk-params"));
    let call = DispatchCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: IndexMap::new(),
        api_base: None,
        api_key: None,
        timeout: None,
        extra_headers: None,
        aspeech: false,
        environment: &speech_environment,
    };
    let request = resolved(call).await;
    assert_eq!(request.api_base.as_deref(), Some("https://global.example"));
    assert_eq!(request.api_key.as_deref(), Some("sk-global-key"));
    let no_globals = SpeechGlobals::default();
    let environment_only = SpeechEnvironment {
        globals: &no_globals,
        environment,
    };
    let call = DispatchCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: IndexMap::new(),
        api_base: None,
        api_key: None,
        timeout: None,
        extra_headers: None,
        aspeech: false,
        environment: &environment_only,
    };
    let request = resolved(call).await;
    assert_eq!(request.api_base.as_deref(), Some("https://env.example"));
    assert_eq!(request.api_key.as_deref(), Some("sk-openai-env"));
    let azure_only = BTreeMap::from([
        (
            String::from("AZURE_API_BASE"),
            String::from("https://env.example"),
        ),
        (String::from("AZURE_API_KEY"), String::from("sk-azure-env")),
    ]);
    let fallback_environment = SpeechEnvironment {
        globals: &no_globals,
        environment: &azure_only,
    };
    let call = DispatchCall {
        model: "tts-1",
        input: "hello",
        voice: None,
        optional_params: &optional,
        litellm_params: IndexMap::new(),
        api_base: None,
        api_key: None,
        timeout: None,
        extra_headers: None,
        aspeech: false,
        environment: &fallback_environment,
    };
    let request = resolved(call).await;
    assert_eq!(request.api_base.as_deref(), Some("https://env.example"));
    assert_eq!(request.api_key.as_deref(), Some("sk-azure-env"));
}

async fn voice_rules(
    globals: &SpeechGlobals,
    environment: &BTreeMap<String, String>,
    optional: &IndexMap<String, Json>,
) {
    let speech_environment = SpeechEnvironment {
        globals,
        environment,
    };
    let text = json("\"alloy\"");
    let named = json(r#"{"name":"echo"}"#);
    let nameless = json("{}");
    let number = json("7");
    let null = json("null");
    let named_request = voice_request(Some(&named), &speech_environment, optional).await;
    assert_eq!(named_request.voice.as_deref(), Some("echo"));
    let nameless_request = voice_request(Some(&nameless), &speech_environment, optional).await;
    assert_eq!(nameless_request.voice, None);
    let number_request = voice_request(Some(&number), &speech_environment, optional).await;
    assert_eq!(number_request.voice, None);
    let text_request = voice_request(Some(&text), &speech_environment, optional).await;
    assert_eq!(text_request.voice.as_deref(), Some("alloy"));
    let null_request = voice_request(Some(&null), &speech_environment, optional).await;
    assert_eq!(null_request.voice, None);
    let wrong = json(r#"{"name":3}"#);
    let call = DispatchCall {
        model: "tts-1",
        input: "hello",
        voice: Some(&wrong),
        optional_params: optional,
        litellm_params: IndexMap::new(),
        api_base: None,
        api_key: None,
        timeout: None,
        extra_headers: None,
        aspeech: false,
        environment: &speech_environment,
    };
    let error = failure(call).await;
    assert_eq!(error.class(), "Exception");
}

async fn voice_request(
    voice: Option<&Json>,
    environment: &SpeechEnvironment<'_>,
    optional: &IndexMap<String, Json>,
) -> HandlerRecord {
    let call = DispatchCall {
        model: "tts-1",
        input: "hello",
        voice,
        optional_params: optional,
        litellm_params: IndexMap::new(),
        api_base: None,
        api_key: None,
        timeout: None,
        extra_headers: None,
        aspeech: false,
        environment,
    };
    resolved(call).await
}

async fn resolved(call: DispatchCall<'_>) -> HandlerRecord {
    let mut handler = RecordingHandler::default();
    dispatch_text_to_speech(call, &mut handler)
        .await
        .expect("the handler answers");
    first(&handler)
}

async fn failure(call: DispatchCall<'_>) -> AzureSpeechError {
    let mut handler = RecordingHandler::default();
    let error = dispatch_text_to_speech(call, &mut handler)
        .await
        .expect_err("the dispatch fails");
    assert!(handler.requests.is_empty());
    error
}

fn first(handler: &RecordingHandler) -> HandlerRecord {
    assert_eq!(handler.requests.len(), 1);
    let request = handler.requests[0].clone();
    request
}

fn binary_speech_body() {
    let mut inputs = Inputs::default();
    let mut logger = Recorder::default();
    let optional = support::pairs(&[
        ("max_retries", json("2")),
        ("extra_headers", json(r#"{"x-sdk":"1"}"#)),
        ("extra_query", json(r#"{"trace":"abc"}"#)),
        ("extra_body", json(r#"{"instructions":"hi"}"#)),
        ("timeout", json("5")),
        ("response_format", json("\"mp3\"")),
        ("speed", json("1.5")),
    ]);
    let call = support::call("tts", "hello", "alloy", optional);
    let request =
        support::start(&mut inputs, &call, &mut logger).expect("the speech request builds");
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/deployments/tts/audio/speech?api-version=2024-06-01&trace=abc"
    );
    assert_eq!(request.deadline, Duration::from_secs(5));
    assert_eq!(support::header(&request, "x-sdk"), Some("1"));
    assert_eq!(
        support::body(&request),
        json(
            r#"{"model":"tts","input":"hello","voice":"alloy","response_format":"mp3","speed":1.5,"instructions":"hi"}"#
        )
    );
    let pre = logger.pre.expect("the pre-call runs");
    assert_eq!(
        pre.data,
        json(
            r#"{"model":"tts","voice":"alloy","extra_query":{"trace":"abc"},"extra_body":{"instructions":"hi"},"timeout":5,"response_format":"mp3","speed":1.5}"#
        )
    );
    let mut timed = support::call("tts", "hello", "alloy", IndexMap::new());
    timed.params.timeout = Some(12.0_f64);
    let timeout_from_params = support::start(&mut inputs, &timed, &mut Recorder::default())
        .expect("the client timeout applies")
        .deadline;
    assert_eq!(timeout_from_params, Duration::from_secs(12));
    let duplicate = support::pairs(&[("model", json("\"other\""))]);
    let call = support::call("tts", "hello", "alloy", duplicate);
    let error = support::start(&mut inputs, &call, &mut Recorder::default())
        .expect_err("a reserved option is a duplicate argument");
    assert_eq!(error.class(), "TypeError");
}

fn azure_speech_supported_params() {
    assert_eq!(
        supported_openai_params(),
        vec![
            String::from("voice"),
            String::from("response_format"),
            String::from("speed"),
        ]
    );
}

fn azure_rate_limit_probe() {
    let call = ProbeCall {
        model: Some("gpt-4o"),
        api_key: API_KEY,
        api_base: Some(API_BASE),
        api_version: Some(API_VERSION),
        timeout: None,
        mode: ProbeMode::Chat,
        messages: None,
    };
    let request = probe_request(&call).expect("the probe request builds");
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-06-01"
    );
    assert_eq!(support::header(&request, "api-key"), Some(API_KEY));
    assert_eq!(
        support::header(&request, "accept"),
        Some("application/json")
    );
    assert_eq!(
        support::body(&request),
        json(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hey"}]}"#)
    );
    let given = json(r#"[{"role":"user","content":"Hi"}]"#);
    let with_messages = ProbeCall {
        model: Some("gpt-4o"),
        api_key: API_KEY,
        api_base: Some(API_BASE),
        api_version: Some(API_VERSION),
        timeout: Some(2.0_f64),
        mode: ProbeMode::Chat,
        messages: Some(&given),
    };
    let request = probe_request(&with_messages).expect("the probe request builds");
    assert_eq!(request.deadline, Duration::from_secs(2));
    assert_eq!(
        support::body(&request),
        json(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hi"}]}"#)
    );
    let found = finish_probe(Some(support::response(
        200,
        &[
            ("x-ratelimit-remaining-requests", "9"),
            ("x-ratelimit-remaining-tokens", "8"),
            ("x-ms-region", "eastus"),
            ("content-type", "application/json"),
        ],
        b"{}",
    )))
    .expect("the rate-limit headers come back");
    assert_eq!(found.len(), 3);
    assert_eq!(found.get("x-ms-region").map(String::as_str), Some("eastus"));
    let missing = finish_probe(None).expect_err("a missing completion fails");
    assert_eq!(missing.class(), "Exception");
    let failed = finish_probe(Some(support::response(429, &[], b"slow down")))
        .expect_err("the status propagates");
    assert_eq!(failed.class(), "APIStatusError");
    assert_eq!(failed.status(), Some(429));
}

fn azure_probe_cloudflare() {
    let gateway = ProbeCall {
        model: Some("gpt-4o"),
        api_key: API_KEY,
        api_base: Some("https://gateway.ai.cloudflare.com/v1/acct/gw/azure"),
        api_version: Some(API_VERSION),
        timeout: None,
        mode: ProbeMode::Chat,
        messages: None,
    };
    let request = probe_request(&gateway).expect("the gateway probe builds");
    assert_eq!(
        request.url.as_str(),
        "https://gateway.ai.cloudflare.com/v1/acct/gw/azure/gpt-4o/chat/completions?api-version=2024-06-01"
    );
    assert_eq!(
        support::body(&request),
        json(r#"{"model":null,"messages":[{"role":"user","content":"Hey"}]}"#)
    );
    let trailing = ProbeCall {
        model: Some("gpt-4o"),
        api_key: API_KEY,
        api_base: Some("https://gateway.ai.cloudflare.com/v1/acct/gw/azure/"),
        api_version: Some(API_VERSION),
        timeout: None,
        mode: ProbeMode::Chat,
        messages: None,
    };
    let request = probe_request(&trailing).expect("the gateway probe builds");
    assert_eq!(
        request.url.as_str(),
        "https://gateway.ai.cloudflare.com/v1/acct/gw/azure/gpt-4o/chat/completions?api-version=2024-06-01"
    );
    let unnamed = ProbeCall {
        model: None,
        api_key: API_KEY,
        api_base: Some("https://gateway.ai.cloudflare.com/v1/acct/gw/azure"),
        api_version: Some(API_VERSION),
        timeout: None,
        mode: ProbeMode::Chat,
        messages: None,
    };
    let request = probe_request(&unnamed).expect("the unnamed gateway probe builds");
    assert_eq!(
        request.url.as_str(),
        "https://gateway.ai.cloudflare.com/v1/acct/gw/azure/None/chat/completions?api-version=2024-06-01"
    );
    let unnamed_azure = ProbeCall {
        model: None,
        api_key: API_KEY,
        api_base: Some(API_BASE),
        api_version: Some(API_VERSION),
        timeout: None,
        mode: ProbeMode::Chat,
        messages: None,
    };
    let error = probe_request(&unnamed_azure).expect_err("a chat probe needs a model");
    assert_eq!(error.class(), "Exception");
    assert_eq!(error.status(), None);
    let image = ProbeCall {
        model: None,
        api_key: API_KEY,
        api_base: Some(API_BASE),
        api_version: Some(API_VERSION),
        timeout: None,
        mode: ProbeMode::ImageGeneration,
        messages: None,
    };
    let request = probe_request(&image).expect("the image probe builds without a model");
    assert_eq!(
        request.url.as_str(),
        "https://example.openai.azure.com/openai/chat/completions?api-version=2024-06-01"
    );
    let absent = ProbeCall {
        model: Some("gpt-4o"),
        api_key: API_KEY,
        api_base: None,
        api_version: Some(API_VERSION),
        timeout: None,
        mode: ProbeMode::Chat,
        messages: None,
    };
    let error = probe_request(&absent).expect_err("a probe needs an api_base");
    assert_eq!(error.class(), "Exception");
    let empty = ProbeCall {
        model: Some("gpt-4o"),
        api_key: "",
        api_base: Some(API_BASE),
        api_version: Some(API_VERSION),
        timeout: None,
        mode: ProbeMode::Chat,
        messages: None,
    };
    let error = probe_request(&empty).expect_err("an empty key cannot authenticate");
    assert_eq!(error.class(), "ValueError");
}
