use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::credentials::source::TokenReply;
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

// The dispatch call every handler case shares, with each case setting only the slots it is about.
struct CallSpec<'data> {
    environment: &'data SpeechEnvironment<'data>,
    optional: &'data IndexMap<String, Json>,
    voice: Option<&'data Json>,
    litellm_params: IndexMap<String, Json>,
    api_base: Option<&'data str>,
    api_key: Option<&'data str>,
    timeout: Option<Duration>,
    extra_headers: Option<&'data BTreeMap<String, String>>,
    aspeech: bool,
}

impl<'data> CallSpec<'data> {
    fn new(
        environment: &'data SpeechEnvironment<'data>,
        optional: &'data IndexMap<String, Json>,
    ) -> Self {
        Self {
            environment,
            optional,
            voice: None,
            litellm_params: IndexMap::new(),
            api_base: None,
            api_key: None,
            timeout: None,
            extra_headers: None,
            aspeech: false,
        }
    }

    fn request(self) -> DispatchCall<'data> {
        DispatchCall {
            model: "tts-1",
            input: "hello",
            voice: self.voice,
            optional_params: self.optional,
            litellm_params: self.litellm_params,
            api_base: self.api_base.map(str::to_owned),
            api_key: self.api_key.map(SecretString::from),
            timeout: self.timeout,
            extra_headers: self.extra_headers,
            aspeech: self.aspeech,
            environment: self.environment,
        }
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
    assert_eq!(
        Json::Object(pre.data),
        json(r#"{"model":"tts","voice":"alloy"}"#)
    );
    assert_eq!(inputs.cache.entries.len(), 1);
    speech_response_bytes();
    credential_paths();
    unusable_headers();
}

fn speech_response_bytes() {
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
    let Err(failed) = finish_audio_speech(support::response(400, &[], b"bad input")) else {
        panic!("a non-2xx reply is the status error");
    };
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
    let globals = speech_globals();
    let environment = speech_variables();
    let optional = support::pairs(&[("speed", json("1.5"))]);
    let speech_environment = SpeechEnvironment {
        globals: &globals,
        environment: &environment,
    };
    let headers = BTreeMap::from([(String::from("x-call"), String::from("1"))]);
    let voice = json(r#"{"name":"alloy"}"#);
    let mut spec = CallSpec::new(&speech_environment, &optional);
    spec.voice = Some(&voice);
    spec.timeout = Some(Duration::from_secs(5));
    spec.extra_headers = Some(&headers);
    spec.aspeech = true;
    let mut handler = RecordingHandler::default();
    dispatch_text_to_speech(spec.request(), &mut handler)
        .await
        .expect("the handler answers");
    assert_native_request(&handler, &headers, &optional);
    resolve_chains(&globals, &environment).await;
    voice_rules(&globals, &environment, &optional).await;
}

fn speech_globals() -> SpeechGlobals {
    SpeechGlobals {
        api_base: Some(String::from("https://global.example")),
        api_key: Some(SecretString::from("sk-global-key")),
        azure_key: Some(SecretString::from("sk-azure-global")),
    }
}

fn speech_variables() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            String::from("AZURE_API_BASE"),
            String::from("https://env.example"),
        ),
        (
            String::from("AZURE_OPENAI_API_KEY"),
            String::from("sk-openai-env"),
        ),
        (String::from("AZURE_API_KEY"), String::from("sk-azure-env")),
    ])
}

fn assert_native_request(
    handler: &RecordingHandler,
    headers: &BTreeMap<String, String>,
    optional: &IndexMap<String, Json>,
) {
    let request = first(handler);
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
    assert_eq!(&request.optional_params, optional);
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
    let explicit = CallSpec {
        api_base: Some("https://explicit.example"),
        api_key: Some("sk-explicit"),
        litellm_params: params.clone(),
        ..CallSpec::new(&speech_environment, &optional)
    };
    assert_chain(explicit, "https://explicit.example", "sk-explicit").await;
    let from_params = CallSpec {
        litellm_params: params,
        ..CallSpec::new(&speech_environment, &optional)
    };
    assert_chain(from_params, "https://params.example", "sk-params").await;
    let from_globals = CallSpec::new(&speech_environment, &optional);
    assert_chain(from_globals, "https://global.example", "sk-global-key").await;
    resolve_env_chains(environment, &optional).await;
}

async fn resolve_env_chains(
    environment: &BTreeMap<String, String>,
    optional: &IndexMap<String, Json>,
) {
    let no_globals = SpeechGlobals::default();
    let without_globals = SpeechEnvironment {
        globals: &no_globals,
        environment,
    };
    let from_environment = CallSpec::new(&without_globals, optional);
    assert_chain(from_environment, "https://env.example", "sk-openai-env").await;
    let azure_only = BTreeMap::from([
        (
            String::from("AZURE_API_BASE"),
            String::from("https://env.example"),
        ),
        (String::from("AZURE_API_KEY"), String::from("sk-azure-env")),
    ]);
    let azure_key_environment = SpeechEnvironment {
        globals: &no_globals,
        environment: &azure_only,
    };
    let from_azure_key = CallSpec::new(&azure_key_environment, optional);
    assert_chain(from_azure_key, "https://env.example", "sk-azure-env").await;
    // an empty variable counts as unset, so neither slot resolves
    let empty = BTreeMap::from([
        (String::from("AZURE_API_BASE"), String::new()),
        (String::from("AZURE_OPENAI_API_KEY"), String::new()),
    ]);
    let empty_environment = SpeechEnvironment {
        globals: &no_globals,
        environment: &empty,
    };
    let request = resolved(CallSpec::new(&empty_environment, optional)).await;
    assert_eq!(request.api_base, None);
    assert_eq!(request.api_key, None);
}

async fn assert_chain(spec: CallSpec<'_>, expected_base: &str, expected_key: &str) {
    let request = resolved(spec).await;
    assert_eq!(request.api_base.as_deref(), Some(expected_base));
    assert_eq!(request.api_key.as_deref(), Some(expected_key));
    assert!(!request.is_async);
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
    voice_values(&speech_environment, optional).await;
    let wrong = json(r#"{"name":3}"#);
    let mut spec = CallSpec::new(&speech_environment, optional);
    spec.voice = Some(&wrong);
    let error = failure(spec).await;
    assert_eq!(error.class(), "Exception");
}

async fn voice_values(environment: &SpeechEnvironment<'_>, optional: &IndexMap<String, Json>) {
    let echo = json(r#"{"name":"echo"}"#);
    let nameless = json("{}");
    let number = json("7");
    let text = json("\"alloy\"");
    let null = json("null");
    assert_eq!(
        voice_of(Some(&echo), environment, optional)
            .await
            .as_deref(),
        Some("echo")
    );
    assert_eq!(voice_of(Some(&nameless), environment, optional).await, None);
    assert_eq!(voice_of(Some(&number), environment, optional).await, None);
    assert_eq!(
        voice_of(Some(&text), environment, optional)
            .await
            .as_deref(),
        Some("alloy")
    );
    assert_eq!(voice_of(Some(&null), environment, optional).await, None);
}

async fn voice_of<'data>(
    voice: Option<&'data Json>,
    environment: &'data SpeechEnvironment<'data>,
    optional: &'data IndexMap<String, Json>,
) -> Option<String> {
    let mut spec = CallSpec::new(environment, optional);
    spec.voice = voice;
    resolved(spec).await.voice
}

async fn resolved(spec: CallSpec<'_>) -> HandlerRecord {
    let mut handler = RecordingHandler::default();
    dispatch_text_to_speech(spec.request(), &mut handler)
        .await
        .expect("the handler answers");
    first(&handler)
}

async fn failure(spec: CallSpec<'_>) -> AzureSpeechError {
    let mut handler = RecordingHandler::default();
    let error = dispatch_text_to_speech(spec.request(), &mut handler)
        .await
        .expect_err("the dispatch fails");
    assert!(handler.requests.is_empty());
    error
}

fn first(handler: &RecordingHandler) -> HandlerRecord {
    assert_eq!(handler.requests.len(), 1);
    handler.requests[0].clone()
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
    logged_speech_options(logger);
    options_from_params();
    duplicate_option();
    base_queries();
    wrong_controls();
}

fn logged_speech_options(logger: Recorder) {
    let pre = logger.pre.expect("the pre-call runs");
    assert_eq!(
        Json::Object(pre.data),
        json(
            r#"{"model":"tts","voice":"alloy","extra_query":{"trace":"abc"},"extra_body":{"instructions":"hi"},"timeout":5,"response_format":"mp3","speed":1.5}"#
        )
    );
}

fn options_from_params() {
    let mut call = support::call("tts", "hello", "alloy", IndexMap::new());
    call.params.timeout = Some(12.0_f64);
    let request = support::start(&mut Inputs::default(), &call, &mut Recorder::default())
        .expect("the client timeout applies");
    assert_eq!(request.deadline, Duration::from_secs(12));
}

fn duplicate_option() {
    let call = support::call(
        "tts",
        "hello",
        "alloy",
        support::pairs(&[("model", json("\"other\""))]),
    );
    let error = support::start(&mut Inputs::default(), &call, &mut Recorder::default())
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
    probe_messages();
    probe_rate_limit_headers();
}

fn probe_messages() {
    let given = json(r#"[{"role":"user","content":"Hi"}]"#);
    let call = ProbeCall {
        model: Some("gpt-4o"),
        api_key: API_KEY,
        api_base: Some(API_BASE),
        api_version: Some(API_VERSION),
        timeout: Some(2.0_f64),
        mode: ProbeMode::Chat,
        messages: Some(&given),
    };
    let request = probe_request(&call).expect("the probe request builds");
    assert_eq!(request.deadline, Duration::from_secs(2));
    assert_eq!(
        support::body(&request),
        json(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"Hi"}]}"#)
    );
}

fn probe_rate_limit_headers() {
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
    let slashed = probe_request(&trailing).expect("the gateway probe builds");
    assert_eq!(
        slashed.url.as_str(),
        "https://gateway.ai.cloudflare.com/v1/acct/gw/azure/gpt-4o/chat/completions?api-version=2024-06-01"
    );
    cloudflare_unnamed();
    probe_requires_options();
}

fn cloudflare_unnamed() {
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
}

fn probe_requires_options() {
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
    let base_error = probe_request(&absent).expect_err("a probe needs an api_base");
    assert_eq!(base_error.class(), "Exception");
    let empty = ProbeCall {
        model: Some("gpt-4o"),
        api_key: "",
        api_base: Some(API_BASE),
        api_version: Some(API_VERSION),
        timeout: None,
        mode: ProbeMode::Chat,
        messages: None,
    };
    let key_error = probe_request(&empty).expect_err("an empty key cannot authenticate");
    assert_eq!(key_error.class(), "ValueError");
}

// the caller's authorization header stands in for the SDK's key, and a call with neither fails as ValueError
fn credential_paths() {
    let auth_headers = support::pairs(&[(
        "extra_headers",
        json(r#"{"authorization":"Bearer caller"}"#),
    )]);
    let mut call = support::call("tts", "hello", "alloy", auth_headers);
    call.api_key = None;
    let request = support::start(&mut Inputs::default(), &call, &mut Recorder::default())
        .expect("the caller's authorization authenticates the call");
    assert_eq!(
        support::header(&request, "authorization"),
        Some("Bearer caller")
    );
    assert_eq!(support::header(&request, "api-key"), None);
    missing_key();
    empty_key();
    provider_tokens();
    caller_auth_headers();
}

// a caller header for an auth name wins over the credential the SDK would add for that name
fn caller_auth_headers() {
    let key_headers = support::pairs(&[("extra_headers", json(r#"{"api-key":"caller-key"}"#))]);
    let key_call = support::call("tts", "hello", "alloy", key_headers);
    let key_request = support::start(&mut Inputs::default(), &key_call, &mut Recorder::default())
        .expect("the caller's api-key authenticates the call");
    assert_eq!(support::header(&key_request, "api-key"), Some("caller-key"));
    let token_headers = support::pairs(&[(
        "extra_headers",
        json(r#"{"authorization":"Bearer caller"}"#),
    )]);
    let mut token_call = support::call("tts", "hello", "alloy", token_headers);
    token_call.api_key = None;
    token_call.provider = Some(support::provider(TokenReply::Text(SecretString::from(
        "ad-token",
    ))));
    let request = support::start(
        &mut Inputs::default(),
        &token_call,
        &mut Recorder::default(),
    )
    .expect("the caller's authorization stands over the token");
    assert_eq!(
        support::header(&request, "authorization"),
        Some("Bearer caller")
    );
}

fn missing_key() {
    let mut call = support::call("tts", "hello", "alloy", IndexMap::new());
    call.api_key = None;
    let error = support::start(&mut Inputs::default(), &call, &mut Recorder::default())
        .expect_err("a keyless azure call cannot authenticate");
    assert_eq!(error.class(), "ValueError");
    assert_eq!(error.status(), None);
}

fn empty_key() {
    let mut call = support::call("tts", "hello", "alloy", IndexMap::new());
    call.api_key = Some(SecretString::from(""));
    let error = support::start(&mut Inputs::default(), &call, &mut Recorder::default())
        .expect_err("an empty key cannot authenticate");
    assert_eq!(error.class(), "ValueError");
}

// an azure_ad_token_provider answers with the bearer token, and anything but a non-empty string fails
fn provider_tokens() {
    let token = TokenReply::Text(SecretString::from("ad-token"));
    let mut call = support::call("tts", "hello", "alloy", IndexMap::new());
    call.api_key = None;
    call.provider = Some(support::provider(token));
    let request = support::start(&mut Inputs::default(), &call, &mut Recorder::default())
        .expect("the token provider authenticates the call");
    assert_eq!(
        support::header(&request, "authorization"),
        Some("Bearer ad-token")
    );
    assert_token_failure(TokenReply::Text(SecretString::from("")), "");
    assert_token_failure(
        TokenReply::NonText {
            type_name: String::from("int"),
        },
        "int",
    );
}

// the SDK names the reply it could not use, but never prints a text reply, which can hold a credential
fn assert_token_failure(reply: TokenReply, returned: &str) {
    let mut call = support::call("tts", "hello", "alloy", IndexMap::new());
    call.api_key = None;
    call.provider = Some(support::provider(reply));
    let error = support::start(&mut Inputs::default(), &call, &mut Recorder::default())
        .expect_err("a token the SDK cannot use fails the call");
    assert_eq!(error.class(), "ValueError");
    assert_eq!(
        error.to_string(),
        format!(
            "Expected `azure_ad_token_provider` argument to return a string but it returned {returned}"
        )
    );
}

// a header the wire cannot carry fails before the send, as the SDK's client construction does
fn unusable_headers() {
    for headers in [json(r#"{"bad header":"v"}"#), json(r#"{"x-bad":"a\nb"}"#)] {
        let call = support::call(
            "tts",
            "hello",
            "alloy",
            support::pairs(&[("extra_headers", headers)]),
        );
        let error = support::start(&mut Inputs::default(), &call, &mut Recorder::default())
            .expect_err("an unusable header cannot be sent");
        assert_eq!(error.class(), "Exception");
    }
}

// a base that already names the deployment keeps its own query, and extra_query overrides a repeated key
fn base_queries() {
    let query_params = support::pairs(&[(
        "extra_query",
        json(r#"{"api-version":"1999-01-01","trace":"abc"}"#),
    )]);
    let mut call = support::call("tts", "hello", "alloy", query_params);
    call.api_base = Some(String::from(
        "https://gateway.example/openai/deployments/tts?foo=bar&",
    ));
    let request = support::start(&mut Inputs::default(), &call, &mut Recorder::default())
        .expect("a deployment endpoint with a query builds");
    assert_eq!(
        request.url.as_str(),
        "https://gateway.example/openai/deployments/tts/audio/speech?foo=bar&api-version=1999-01-01&trace=abc"
    );
}

// the caller controls the SDK takes are not body fields, and a wrong type names the option
fn wrong_controls() {
    let body = support::pairs(&[("extra_body", json("\"hi\""))]);
    let body_call = support::call("tts", "hello", "alloy", body);
    let body_error = support::start(&mut Inputs::default(), &body_call, &mut Recorder::default())
        .expect_err("a text extra_body cannot merge");
    assert_eq!(body_error.class(), "RuntimeError");
    assert!(
        body_error
            .to_string()
            .contains("Unexpected JSON data type, string, cannot merge with `extra_body`")
    );
    let headers = support::pairs(&[("extra_headers", json("\"x\""))]);
    let headers_call = support::call("tts", "hello", "alloy", headers);
    let headers_error = support::start(
        &mut Inputs::default(),
        &headers_call,
        &mut Recorder::default(),
    )
    .expect_err("a text extra_headers is not a mapping");
    assert!(
        headers_error
            .to_string()
            .contains("is not a mapping of strings")
    );
    let timeout = support::pairs(&[("timeout", json("\"5\""))]);
    let timeout_call = support::call("tts", "hello", "alloy", timeout);
    let timeout_error = support::start(
        &mut Inputs::default(),
        &timeout_call,
        &mut Recorder::default(),
    )
    .expect_err("a text timeout is not a number of seconds");
    assert!(
        timeout_error
            .to_string()
            .contains("is not a number of seconds")
    );
}
