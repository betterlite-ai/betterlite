use std::collections::BTreeMap;

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use futures_util::StreamExt as _;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_stream_errors::error::{Reported, StreamingError};
use crate::azure_stream_errors::prepare::{
    ASYNC_CLIENT_MESSAGE, SYNC_CLIENT_MESSAGE, StreamCredential, unused_client_params,
};
use crate::azure_stream_errors::records::StreamCall;
use crate::azure_stream_errors::stream::{AzureChatStream, ChatStream, WrapperHeaders};
use crate::azure_stream_errors::{async_streaming, streaming};
use crate::endpoints::projection::ClientKind;

use super::fixture::{
    CallFixture, Recorder, ScriptedSource, ScriptedSources, azure_failure, block_on,
    callable_provider, gets_of, requests_of, runtime, scripted_source, sse_payload,
};
use super::stream_failure_variants::{
    assert_asked_stream_options, assert_async_refuses_sync, assert_response_fallback,
    assert_sync_original_class,
};
use super::stream_read_failures::{assert_absent_stream_options, assert_stream_failures};
use super::stream_send_failures::{
    assert_empty_stream_options, assert_explicit_headers, assert_send_failure,
};

#[rstest]
#[case::stream_request(stream_request)]
#[case::stream_options(stream_options)]
#[case::stream_http_failure(stream_http_failure)]
#[case::stream_header_difference(stream_header_difference)]
#[case::sync_unused_preparation(sync_unused_preparation)]
#[case::stream_logging(stream_logging)]
#[case::stream_wrong_client(stream_wrong_client)]
fn azure_stream_errors_cases(#[case] scenario: fn()) {
    scenario();
}

fn plain_sources() -> ScriptedSources {
    ScriptedSources::new(200, "{}", BTreeMap::new())
}

fn collected(mut items: ChatStream) -> Vec<StreamChunk> {
    block_on(async move {
        let mut values = Vec::new();
        while let Some(item) = items.next().await {
            values.push(item.expect("chunk"));
        }
        values
    })
}

fn assert_sync_get(source: &ScriptedSource, call: &StreamCall<'_>) {
    let gets = gets_of(source);
    assert_eq!(gets.len(), 1);
    assert_eq!(gets[0].api_version, "2024-02-01");
    assert_eq!(gets[0].api_base, "https://unit.test");
    assert_eq!(gets[0].api_key.as_deref(), Some("call-key"));
    assert_eq!(gets[0].model, "deployment");
    assert!(!gets[0].is_async);
    assert!(!gets[0].client_present);

    let requests = requests_of(source);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout, Some(30.0));
    assert_eq!(requests[0].data, *call.data);
    assert!(requests[0].data.get("stream").is_none());
}

fn assert_stream_and_pre(stream: &AzureChatStream, recorder: &Recorder, call: &StreamCall<'_>) {
    assert_eq!(stream.custom_llm_provider, "azure");
    assert_eq!(stream.model, "deployment");
    assert_eq!(recorder.pre.len(), 1);
    let pre = &recorder.pre[0];
    assert_eq!(pre.input, call.data["messages"]);
    assert_eq!(
        pre.api_key.as_ref().map(|key| key.expose_secret()),
        Some("client-key")
    );
    let headers = pre.additional_args.headers.as_ref().expect("headers");
    assert_eq!(
        headers.get("api_key"),
        Some(&Json::Text("call-key".to_owned()))
    );
    assert_eq!(headers.get("azure_ad_token"), Some(&Json::Null));
    assert_eq!(
        pre.additional_args.api_base.as_deref(),
        Some("https://unit.test")
    );
    assert_eq!(
        pre.additional_args.passthrough.get("acompletion"),
        Some(&Json::Bool(true))
    );
    assert_eq!(
        pre.additional_args.complete_input_dict,
        Some(RawBody::Object(call.data.clone()))
    );
}

fn stream_request() {
    let source = scripted_source(
        ClientKind::AzureOpenAi,
        &[("x-ms-request-id", "req-1")],
        sse_payload(),
    );
    let sources = plain_sources();
    let fixture = CallFixture::default();
    let call = fixture.call();
    let mut recorder = Recorder::default();
    let stream = block_on(streaming(
        &source,
        &sources,
        &call,
        &mut recorder,
        runtime(),
    ))
    .expect("stream opens");
    assert_sync_get(&source, &call);
    assert_stream_and_pre(&stream, &recorder, &call);
    let chunks = collected(stream.chunks);
    assert!(!chunks.is_empty());
    assert_eq!(chunks[0].choices[0].delta.content.as_deref(), Some("Hello"));
}

fn stream_options() {
    let source = scripted_source(
        ClientKind::AzureOpenAi,
        &[("x-ms-request-id", "req-1")],
        sse_payload(),
    );
    let sources = plain_sources();
    let mut recorder = Recorder::default();
    assert_empty_stream_options(&source, &sources, &mut recorder);
    assert_absent_stream_options(&source, &sources, &mut recorder);
    assert_asked_stream_options(&source, &sources, &mut recorder);
    assert_eq!(recorder.response_headers.len(), 3);
    assert_eq!(
        recorder.response_headers[0]
            .get("x-ms-request-id")
            .map(String::as_str),
        Some("req-1")
    );
}

fn stream_http_failure() {
    let sources = plain_sources();
    let fixture = CallFixture::default();
    let call = fixture.call();
    let mut recorder = Recorder::default();
    assert_send_failure(&sources, &call, &mut recorder);
    assert_sync_original_class(&sources, &call, &mut recorder);
    assert_stream_failures(&sources, &call, &mut recorder);
    assert_response_fallback(&call, &mut recorder);
    assert_explicit_headers(&call, &mut recorder);
}

fn assert_processed_headers(stream: &AzureChatStream) {
    let WrapperHeaders::Processed(processed) = &stream.response_headers else {
        panic!("sync passes processed headers");
    };
    assert_eq!(
        processed
            .get("llm_provider-x-ratelimit-limit-requests")
            .map(String::as_str),
        Some("7")
    );
    assert_eq!(
        processed
            .get("llm_provider-x-custom-name")
            .map(String::as_str),
        Some("MiXeD")
    );
    assert_eq!(
        processed
            .get("x-ratelimit-limit-requests")
            .map(String::as_str),
        Some("7")
    );
    assert_eq!(
        stream.additional_headers.get("llm_provider-x-custom-name"),
        Some(&Json::Text("MiXeD".to_owned()))
    );
}

fn assert_raw_headers(stream: &AzureChatStream) {
    let WrapperHeaders::Raw(raw) = &stream.response_headers else {
        panic!("async passes raw headers");
    };
    assert_eq!(raw.get("x-custom-name").map(String::as_str), Some("MiXeD"));
    assert_eq!(
        stream.additional_headers.get("llm_provider-x-custom-name"),
        Some(&Json::Text("MiXeD".to_owned()))
    );
}

fn stream_header_difference() {
    let sources = plain_sources();
    let fixture = CallFixture::default();
    let call = fixture.call();
    let headers = [
        ("x-ratelimit-limit-requests", "7"),
        ("x-custom-name", "MiXeD"),
    ];
    let mut recorder = Recorder::default();
    let source = scripted_source(ClientKind::AzureOpenAi, &headers, sse_payload());
    let sync_stream = block_on(streaming(
        &source,
        &sources,
        &call,
        &mut recorder,
        runtime(),
    ))
    .expect("sync opens");
    assert_processed_headers(&sync_stream);
    let async_source = scripted_source(ClientKind::AsyncAzureOpenAi, &headers, sse_payload());
    let async_stream = block_on(async_streaming(
        &async_source,
        &call,
        &mut recorder,
        runtime(),
    ))
    .expect("async opens");
    assert_raw_headers(&async_stream);
    assert_eq!(recorder.response_headers.len(), 2);
    assert_eq!(
        recorder.response_headers[1]
            .get("x-custom-name")
            .map(String::as_str),
        Some("MiXeD")
    );
}

fn assert_unused_map(sources: &ScriptedSources) {
    let mut fixture = CallFixture::default();
    fixture.api_base = "https://unit.test/openai/deployments/deployment".to_owned();
    fixture.max_retries = 3;
    fixture.timeout = Some(12.5);
    let call = fixture.call();
    let params = unused_client_params(&call, sources).expect("map builds");
    assert_eq!(params.api_version, "2024-02-01");
    assert_eq!(params.endpoint.azure_endpoint, None);
    assert_eq!(
        params.endpoint.base_url.as_deref(),
        Some("https://unit.test/openai/deployments/deployment")
    );
    assert_eq!(params.azure_deployment, "deployment");
    assert!(!params.http_client.is_async);
    assert!(params.http_client.shared);
    assert_eq!(params.max_retries, 3);
    assert_eq!(params.timeout, Some(12.5));
    assert!(matches!(&params.credential, StreamCredential::ApiKey));
    assert!(params.logged_token().is_none());
}

fn assert_plain_token(sources: &ScriptedSources) {
    let mut token_call = CallFixture::default();
    token_call.api_key = None;
    token_call.azure_ad_token = Some(SecretString::from("plain-token"));
    let token_fixture = token_call.call();
    let params = unused_client_params(&token_fixture, sources).expect("token map builds");
    assert!(matches!(
        &params.credential,
        StreamCredential::AzureAdToken(_)
    ));
    assert_eq!(
        params.logged_token().map(|token| token.expose_secret()),
        Some("plain-token")
    );
    assert!(sources.forms().is_empty());
}

fn oidc_sources() -> ScriptedSources {
    ScriptedSources::new(
        200,
        "{\"access_token\":\"exchanged-token\",\"expires_in\":3600}",
        BTreeMap::from([(
            "env/ASSERTION".to_owned(),
            SecretString::from("assertion-A"),
        )]),
    )
}

fn assert_oidc_exchange() {
    let mut oidc_call = CallFixture::default();
    oidc_call.api_key = None;
    oidc_call.azure_ad_token = Some(SecretString::from("oidc/env/ASSERTION"));
    let oidc = oidc_sources();
    let oidc_fixture = oidc_call.call();
    let source = scripted_source(ClientKind::AzureOpenAi, &[], sse_payload());
    let mut recorder = Recorder::default();
    block_on(streaming(
        &source,
        &oidc,
        &oidc_fixture,
        &mut recorder,
        runtime(),
    ))
    .expect("oidc stream opens");
    let forms = oidc.forms();
    assert_eq!(forms.len(), 1);
    assert_eq!(
        forms[0].0,
        "https://login.microsoftonline.com/tenant/oauth2/v2.0/token"
    );
    assert!(
        forms[0]
            .1
            .contains(&("client_assertion".to_owned(), "assertion-A".to_owned()))
    );
    let headers = recorder.pre[0]
        .additional_args
        .headers
        .as_ref()
        .expect("headers");
    assert_eq!(
        headers.get("azure_ad_token"),
        Some(&Json::Text("exchanged-token".to_owned()))
    );
    assert_eq!(gets_of(&source)[0].api_key, None);
}

fn assert_provider(sources: &ScriptedSources) {
    let mut provider_call = CallFixture::default();
    provider_call.api_key = None;
    provider_call.provider = Some(callable_provider(42));
    let provider_fixture = provider_call.call();
    let params = unused_client_params(&provider_fixture, sources).expect("provider map builds");
    assert!(matches!(
        &params.credential,
        StreamCredential::AzureAdTokenProvider(42)
    ));
}

fn assert_oidc_failure() {
    let mut oidc_call = CallFixture::default();
    oidc_call.api_key = None;
    oidc_call.azure_ad_token = Some(SecretString::from("oidc/env/ASSERTION"));
    let failing_sources = ScriptedSources::new(
        400,
        "{\"error\":\"invalid_client\",\"error_description\":\"bad assertion\"}",
        BTreeMap::from([(
            "env/ASSERTION".to_owned(),
            SecretString::from("assertion-A"),
        )]),
    );
    let oidc_fixture = oidc_call.call();
    let source = scripted_source(ClientKind::AzureOpenAi, &[], sse_payload());
    let mut recorder = Recorder::default();
    let error = block_on(streaming(
        &source,
        &failing_sources,
        &oidc_fixture,
        &mut recorder,
        runtime(),
    ))
    .err()
    .expect("the exchange failure surfaces");
    let failure = azure_failure(error);
    assert_eq!(failure.class, "AzureOpenAIError");
    assert_eq!(failure.status, Reported::Present(Some(400)));
    assert!(gets_of(&source).is_empty());
}

fn assert_malformed_exchange() {
    let mut oidc_call = CallFixture::default();
    oidc_call.api_key = None;
    oidc_call.azure_ad_token = Some(SecretString::from("oidc/env/ASSERTION"));
    let oidc_fixture = oidc_call.call();
    let malformed = ScriptedSources::new(200, "{}", BTreeMap::new());
    let mut recorder = Recorder::default();
    let error = block_on(streaming(
        &scripted_source(ClientKind::AzureOpenAi, &[], sse_payload()),
        &malformed,
        &oidc_fixture,
        &mut recorder,
        runtime(),
    ))
    .err()
    .expect("a body without an access token fails");
    assert_eq!(azure_failure(error).status, Reported::Present(Some(422)));
}

fn sync_unused_preparation() {
    let sources = plain_sources();
    assert_unused_map(&sources);
    assert_plain_token(&sources);
    assert_oidc_exchange();
    assert_provider(&sources);
    assert_oidc_failure();
    assert_malformed_exchange();
}

fn assert_oidc_pre(source: &ScriptedSource, call: &StreamCall<'_>) {
    let oidc = oidc_sources();
    let mut recorder = Recorder::default();
    block_on(streaming(source, &oidc, call, &mut recorder, runtime())).expect("sync stream opens");
    let headers = recorder.pre[0]
        .additional_args
        .headers
        .as_ref()
        .expect("headers");
    assert_eq!(headers.get("api_key"), Some(&Json::Null));
    assert_eq!(
        headers.get("azure_ad_token"),
        Some(&Json::Text("exchanged-token".to_owned()))
    );
}

fn assert_sync_pre_refusal(
    source: &ScriptedSource,
    sources: &ScriptedSources,
    call: &StreamCall<'_>,
) {
    let mut refused = Recorder {
        pre: Vec::new(),
        response_headers: Vec::new(),
        fail_pre: true,
    };
    let error = block_on(streaming(source, sources, call, &mut refused, runtime()))
        .err()
        .expect("a refused pre_call fails the sync call");
    assert!(matches!(
        error,
        StreamingError::Logging(PrePostError::CallbackFailed { .. })
    ));
}

fn assert_async_pre_refusal(source: &ScriptedSource, call: &StreamCall<'_>) {
    let mut refused = Recorder {
        pre: Vec::new(),
        response_headers: Vec::new(),
        fail_pre: true,
    };
    let error = block_on(async_streaming(source, call, &mut refused, runtime()))
        .err()
        .expect("a refused pre_call fails the async call");
    let failure = azure_failure(error);
    assert_eq!(failure.status, Reported::Present(Some(500)));
    assert_eq!(
        failure.message,
        Reported::Present(Some("pre_call refused".to_owned()))
    );
}

fn stream_logging() {
    let sources = plain_sources();
    let source = scripted_source(ClientKind::AzureOpenAi, &[], sse_payload());
    let mut fixture = CallFixture::default();
    fixture.api_key = None;
    fixture.azure_ad_token = Some(SecretString::from("oidc/env/ASSERTION"));
    let call = fixture.call();
    assert_oidc_pre(&source, &call);
    assert_sync_pre_refusal(&source, &sources, &call);
    assert_async_pre_refusal(&source, &call);
}

fn assert_sync_refuses_async(
    sources: &ScriptedSources,
    call: &StreamCall<'_>,
    recorder: &mut Recorder,
) {
    let source = scripted_source(ClientKind::AsyncAzureOpenAi, &[], sse_payload());
    let error = block_on(streaming(&source, sources, call, recorder, runtime()))
        .err()
        .expect("sync refuses an async client");
    let failure = azure_failure(error);
    assert_eq!(failure.class, "AzureOpenAIError");
    assert_eq!(failure.status, Reported::Present(Some(500)));
    assert_eq!(
        failure.message,
        Reported::Present(Some(SYNC_CLIENT_MESSAGE.to_owned()))
    );
    assert!(recorder.pre.is_empty());
    assert!(requests_of(&source).is_empty());
}

fn assert_sync_accepts_open_ai(
    sources: &ScriptedSources,
    call: &StreamCall<'_>,
    recorder: &mut Recorder,
) {
    let source = scripted_source(ClientKind::OpenAi, &[], sse_payload());
    block_on(streaming(&source, sources, call, recorder, runtime()))
        .expect("sync accepts an OpenAI client");
}

fn assert_other_client(sources: &ScriptedSources, call: &StreamCall<'_>, recorder: &mut Recorder) {
    let source = scripted_source(ClientKind::Other, &[], sse_payload());
    let sync_error = azure_failure(
        block_on(streaming(&source, sources, call, recorder, runtime()))
            .err()
            .expect("sync refuses any other client"),
    );
    assert_eq!(
        sync_error.message,
        Reported::Present(Some(SYNC_CLIENT_MESSAGE.to_owned()))
    );
    let async_error = azure_failure(
        block_on(async_streaming(&source, call, recorder, runtime()))
            .err()
            .expect("async refuses any other client"),
    );
    assert_eq!(
        async_error.message,
        Reported::Present(Some(ASYNC_CLIENT_MESSAGE.to_owned()))
    );
}

fn assert_async_accepts(call: &StreamCall<'_>, recorder: &mut Recorder) {
    let source = scripted_source(ClientKind::AsyncOpenAi, &[], sse_payload());
    block_on(async_streaming(&source, call, recorder, runtime()))
        .expect("async accepts an AsyncOpenAI client");
}

fn stream_wrong_client() {
    let sources = plain_sources();
    let fixture = CallFixture::default();
    let call = fixture.call();
    let mut recorder = Recorder::default();
    assert_sync_refuses_async(&sources, &call, &mut recorder);
    assert_sync_accepts_open_ai(&sources, &call, &mut recorder);
    assert_async_refuses_sync(&call, &mut recorder);
    assert_async_accepts(&call, &mut recorder);
    assert_other_client(&sources, &call, &mut recorder);
}
