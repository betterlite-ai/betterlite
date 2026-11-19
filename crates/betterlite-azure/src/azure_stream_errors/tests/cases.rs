use std::collections::BTreeMap;

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use futures_util::StreamExt as _;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_stream_errors::error::{Raised, Reported, StreamingError};
use crate::azure_stream_errors::prepare::{
    ASYNC_CLIENT_MESSAGE, SYNC_CLIENT_MESSAGE, StreamCredential,
};
use crate::azure_stream_errors::stream::WrapperHeaders;
use crate::azure_stream_errors::{async_streaming, streaming};
use crate::endpoints::projection::ClientKind;

use super::fixture::{
    CallFixture, Recorder, ScriptedSources, azure_failure, block_on, callable_provider, chat_data,
    failing_source, gets_of, raised_with, requests_of, runtime, scripted_source, secret_headers,
    sse_payload,
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

    let gets = gets_of(&source);
    assert_eq!(gets.len(), 1);
    assert_eq!(gets[0].api_version, "2024-02-01");
    assert_eq!(gets[0].api_base, "https://unit.test");
    assert_eq!(gets[0].api_key.as_deref(), Some("call-key"));
    assert_eq!(gets[0].model, "deployment");
    assert!(!gets[0].is_async);
    assert!(!gets[0].client_present);

    let requests = requests_of(&source);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout, Some(30.0));
    assert_eq!(requests[0].data, *call.data);
    assert!(requests[0].data.get("stream").is_none());

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

    let chunks = block_on(async {
        let mut chunks = stream.chunks;
        let mut collected = Vec::new();
        while let Some(item) = chunks.next().await {
            collected.push(item.expect("chunk"));
        }
        collected
    });
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

    let mut empty = CallFixture::default();
    empty
        .data
        .insert("stream_options".to_owned(), Json::Object(IndexMap::new()));
    let empty_call = empty.call();
    let empty_stream = block_on(streaming(
        &source,
        &sources,
        &empty_call,
        &mut recorder,
        runtime(),
    ))
    .expect("empty stream options open");
    assert_eq!(empty_stream.stream_options, None);

    let absent = CallFixture::default();
    let absent_call = absent.call();
    let absent_stream = block_on(streaming(
        &source,
        &sources,
        &absent_call,
        &mut recorder,
        runtime(),
    ))
    .expect("absent stream options open");
    assert_eq!(absent_stream.stream_options, None);

    let mut asked = CallFixture::default();
    asked.data.insert(
        "stream_options".to_owned(),
        Json::Object(IndexMap::from([(
            "include_usage".to_owned(),
            Json::Bool(true),
        )])),
    );
    let asked_call = asked.call();
    let asked_stream = block_on(streaming(
        &source,
        &sources,
        &asked_call,
        &mut recorder,
        runtime(),
    ))
    .expect("asked stream options open");
    assert_eq!(
        asked_stream
            .stream_options
            .as_ref()
            .map(|options| options.include_usage),
        Some(true)
    );

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

    let source = failing_source(
        ClientKind::AzureOpenAi,
        Raised::azure(400, "bad request".to_owned()),
    );
    let fixture = CallFixture::default();
    let call = fixture.call();
    let mut recorder = Recorder::default();
    let error = block_on(streaming(
        &source,
        &sources,
        &call,
        &mut recorder,
        runtime(),
    ))
    .expect_err("sync propagates the send failure");
    let failure = azure_failure(error);
    assert_eq!(failure.class, "AzureOpenAIError");
    assert_eq!(failure.status, Reported::Present(Some(400)));
    assert_eq!(
        failure.message,
        Reported::Present(Some("bad request".to_owned()))
    );

    let source = failing_source(
        ClientKind::AzureOpenAi,
        Raised::plain("socket closed".to_owned()),
    );
    let error = block_on(streaming(
        &source,
        &sources,
        &call,
        &mut recorder,
        runtime(),
    ))
    .expect_err("sync keeps the original class");
    let failure = azure_failure(error);
    assert_eq!(failure.class, "Exception");
    assert_eq!(failure.status, Reported::Absent);

    let source = failing_source(
        ClientKind::AsyncAzureOpenAi,
        Raised::plain("socket closed".to_owned()),
    );
    let error = block_on(async_streaming(&source, &call, &mut recorder, runtime()))
        .expect_err("async wraps the send failure");
    let failure = azure_failure(error);
    assert_eq!(failure.class, "AzureOpenAIError");
    assert_eq!(failure.status, Reported::Present(Some(500)));
    assert_eq!(
        failure.message,
        Reported::Present(Some("socket closed".to_owned()))
    );

    let source = failing_source(
        ClientKind::AsyncAzureOpenAi,
        Raised::raised(
            "ScriptedError",
            Reported::Present(None),
            Reported::Present(None),
            "".to_owned(),
        ),
    );
    let error = block_on(async_streaming(&source, &call, &mut recorder, runtime()))
        .expect_err("explicit None stays None");
    let failure = azure_failure(error);
    assert_eq!(failure.status, Reported::Present(None));
    assert_eq!(failure.message, Reported::Present(None));

    let fallback = secret_headers(&[("retry-after", "9")]);
    let mut response_only = Raised::plain("sent".to_owned());
    response_only.response_headers = Some(fallback.clone());
    let source = failing_source(ClientKind::AsyncAzureOpenAi, response_only);
    let failure = azure_failure(
        block_on(async_streaming(&source, &call, &mut recorder, runtime()))
            .expect_err("response headers fall back"),
    );
    assert_eq!(failure.headers, Some(fallback));

    let empty = SecretHeaders(BTreeMap::new());
    let mut explicit = Raised::plain("sent".to_owned());
    explicit.headers = Some(empty.clone());
    explicit.response_headers = Some(secret_headers(&[("retry-after", "9")]));
    let source = failing_source(ClientKind::AsyncAzureOpenAi, explicit);
    let failure = azure_failure(
        block_on(async_streaming(&source, &call, &mut recorder, runtime()))
            .expect_err("explicit empty headers suppress the fallback"),
    );
    assert_eq!(failure.headers, Some(empty));

    let source = scripted_source(
        ClientKind::AzureOpenAi,
        &[],
        vec![Err("the connection dropped".to_owned())],
    );
    let stream = block_on(streaming(
        &source,
        &sources,
        &call,
        &mut recorder,
        runtime(),
    ))
    .expect("the stream opens before its bytes fail");
    let error = block_on(async { stream.chunks.next().await })
        .expect("one failing item")
        .expect_err("the read failure is reported");
    let failure = azure_failure(error);
    assert_eq!(failure.class, "APIError");
    assert_eq!(failure.status, Reported::Absent);
    assert_eq!(failure.message, Reported::Absent);
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
    let WrapperHeaders::Processed(processed) = &sync_stream.response_headers else {
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
        sync_stream
            .additional_headers
            .get("llm_provider-x-custom-name"),
        Some(&Json::Text("MiXeD".to_owned()))
    );

    let source = scripted_source(ClientKind::AsyncAzureOpenAi, &headers, sse_payload());
    let async_stream =
        block_on(async_streaming(&source, &call, &mut recorder, runtime())).expect("async opens");
    let WrapperHeaders::Raw(raw) = &async_stream.response_headers else {
        panic!("async passes raw headers");
    };
    assert_eq!(raw.get("x-custom-name").map(String::as_str), Some("MiXeD"));
    assert_eq!(
        async_stream
            .additional_headers
            .get("llm_provider-x-custom-name"),
        Some(&Json::Text("MiXeD".to_owned()))
    );
    assert_eq!(recorder.response_headers.len(), 2);
    assert_eq!(
        recorder.response_headers[1]
            .get("x-custom-name")
            .map(String::as_str),
        Some("MiXeD")
    );
}

fn sync_unused_preparation() {
    use crate::azure_stream_errors::prepare::unused_client_params;

    let sources = plain_sources();
    let mut fixture = CallFixture::default();
    fixture.api_base = "https://unit.test/openai/deployments/deployment".to_owned();
    fixture.max_retries = 3;
    fixture.timeout = Some(12.5);
    let call = fixture.call();
    let params = unused_client_params(&call, &sources).expect("map builds");
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
    assert_eq!(params.credential, StreamCredential::ApiKey);
    assert_eq!(params.logged_token(), None);

    let mut token_call = CallFixture::default();
    token_call.api_key = None;
    token_call.azure_ad_token = Some(SecretString::from("plain-token"));
    let token_fixture = token_call.call();
    let params = unused_client_params(&token_fixture, &sources).expect("token map builds");
    assert_eq!(
        params.credential,
        StreamCredential::AzureAdToken(SecretString::from("plain-token"))
    );
    assert_eq!(
        params.logged_token().map(|token| token.expose_secret()),
        Some("plain-token")
    );
    assert!(sources.forms().is_empty());

    let mut oidc_call = CallFixture::default();
    oidc_call.api_key = None;
    oidc_call.azure_ad_token = Some(SecretString::from("oidc/env/ASSERTION"));
    let assertions = BTreeMap::from([(
        "env/ASSERTION".to_owned(),
        SecretString::from("assertion-A"),
    )]);
    let oidc_sources = ScriptedSources::new(
        200,
        "{\"access_token\":\"exchanged-token\",\"expires_in\":3600}",
        assertions,
    );
    let oidc_fixture = oidc_call.call();
    let source = scripted_source(ClientKind::AzureOpenAi, &[], sse_payload());
    let mut recorder = Recorder::default();
    block_on(streaming(
        &source,
        &oidc_sources,
        &oidc_fixture,
        &mut recorder,
        runtime(),
    ))
    .expect("oidc stream opens");
    let forms = oidc_sources.forms();
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

    let mut provider_call = CallFixture::default();
    provider_call.api_key = None;
    provider_call.provider = Some(callable_provider(42));
    let provider_fixture = provider_call.call();
    let params = unused_client_params(&provider_fixture, &sources).expect("provider map builds");
    assert_eq!(
        params.credential,
        StreamCredential::AzureAdTokenProvider(42)
    );

    let failing_sources = ScriptedSources::new(
        400,
        "{\"error\":\"invalid_client\",\"error_description\":\"bad assertion\"}",
        BTreeMap::from([(
            "env/ASSERTION".to_owned(),
            SecretString::from("assertion-A"),
        )]),
    );
    let source = scripted_source(ClientKind::AzureOpenAi, &[], sse_payload());
    let error = block_on(streaming(
        &source,
        &failing_sources,
        &oidc_fixture,
        &mut recorder,
        runtime(),
    ))
    .expect_err("the exchange failure surfaces");
    let failure = azure_failure(error);
    assert_eq!(failure.class, "AzureOpenAIError");
    assert_eq!(failure.status, Reported::Present(Some(400)));
    assert!(gets_of(&source).is_empty());

    let malformed = ScriptedSources::new(200, "{}", BTreeMap::new());
    let error = block_on(streaming(
        &scripted_source(ClientKind::AzureOpenAi, &[], sse_payload()),
        &malformed,
        &oidc_fixture,
        &mut recorder,
        runtime(),
    ))
    .expect_err("a body without an access token fails");
    assert_eq!(azure_failure(error).status, Reported::Present(Some(422)));
}

fn stream_logging() {
    let sources = plain_sources();
    let source = scripted_source(ClientKind::AzureOpenAi, &[], sse_payload());
    let mut fixture = CallFixture::default();
    fixture.api_key = None;
    fixture.azure_ad_token = Some(SecretString::from("oidc/env/ASSERTION"));
    let oidc_sources = ScriptedSources::new(
        200,
        "{\"access_token\":\"exchanged-token\",\"expires_in\":3600}",
        BTreeMap::from([(
            "env/ASSERTION".to_owned(),
            SecretString::from("assertion-A"),
        )]),
    );
    let call = fixture.call();
    let mut recorder = Recorder::default();
    block_on(streaming(
        &source,
        &oidc_sources,
        &call,
        &mut recorder,
        runtime(),
    ))
    .expect("sync stream opens");
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

    let mut refused = Recorder {
        fail_pre: true,
        ..Recorder::default()
    };
    let error = block_on(streaming(&source, &sources, &call, &mut refused, runtime()))
        .expect_err("a refused pre_call fails the sync call");
    assert!(matches!(
        error,
        StreamingError::Logging(PrePostError::CallbackFailed { .. })
    ));

    let mut refused = Recorder {
        fail_pre: true,
        ..Recorder::default()
    };
    let error = block_on(async_streaming(&source, &call, &mut refused, runtime()))
        .expect_err("a refused pre_call fails the async call");
    let failure = azure_failure(error);
    assert_eq!(failure.status, Reported::Present(Some(500)));
    assert_eq!(
        failure.message,
        Reported::Present(Some("pre_call refused".to_owned()))
    );
}

fn stream_wrong_client() {
    let sources = plain_sources();
    let fixture = CallFixture::default();
    let call = fixture.call();
    let mut recorder = Recorder::default();

    let source = scripted_source(ClientKind::AsyncAzureOpenAi, &[], sse_payload());
    let error = block_on(streaming(
        &source,
        &sources,
        &call,
        &mut recorder,
        runtime(),
    ))
    .expect_err("sync refuses an async client");
    let failure = azure_failure(error);
    assert_eq!(failure.class, "AzureOpenAIError");
    assert_eq!(failure.status, Reported::Present(Some(500)));
    assert_eq!(
        failure.message,
        Reported::Present(Some(SYNC_CLIENT_MESSAGE.to_owned()))
    );
    assert!(recorder.pre.is_empty());
    assert!(requests_of(&source).is_empty());

    let source = scripted_source(ClientKind::OpenAi, &[], sse_payload());
    block_on(streaming(
        &source,
        &sources,
        &call,
        &mut recorder,
        runtime(),
    ))
    .expect("sync accepts an OpenAI client");

    let source = scripted_source(ClientKind::AzureOpenAi, &[], sse_payload());
    let error = block_on(async_streaming(&source, &call, &mut recorder, runtime()))
        .expect_err("async refuses a sync client");
    let failure = azure_failure(error);
    assert_eq!(failure.class, "AzureOpenAIError");
    assert_eq!(failure.status, Reported::Present(Some(500)));
    assert_eq!(
        failure.message,
        Reported::Present(Some(ASYNC_CLIENT_MESSAGE.to_owned()))
    );

    let source = scripted_source(ClientKind::AsyncOpenAi, &[], sse_payload());
    block_on(async_streaming(&source, &call, &mut recorder, runtime()))
        .expect("async accepts an AsyncOpenAI client");

    let source = scripted_source(ClientKind::Other, &[], sse_payload());
    let sync_error = azure_failure(
        block_on(streaming(
            &source,
            &sources,
            &call,
            &mut recorder,
            runtime(),
        ))
        .expect_err("sync refuses any other client"),
    );
    assert_eq!(
        sync_error.message,
        Reported::Present(Some(SYNC_CLIENT_MESSAGE.to_owned()))
    );
    let async_error = azure_failure(
        block_on(async_streaming(&source, &call, &mut recorder, runtime()))
            .expect_err("async refuses any other client"),
    );
    assert_eq!(
        async_error.message,
        Reported::Present(Some(ASYNC_CLIENT_MESSAGE.to_owned()))
    );
}
