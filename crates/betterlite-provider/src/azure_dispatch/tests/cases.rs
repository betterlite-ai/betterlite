use std::collections::BTreeSet;

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialSettings;
use betterlite_config::sections::openai_provider::Settings as OpenAiProviderSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::{CatalogCapabilities, OpenAiSettings};
use betterlite_openai::text_completion_policy::TextCompletionSettings;
use betterlite_prompts::arguments::json::{Json, Object};
use http::HeaderMap;
use secrecy::ExposeSecret as _;

use crate::azure_dispatch::call::{
    Composition, Credentials, Globals, Operation, Request, SuppliedClient, TextCall,
};
use crate::azure_dispatch::compose::{ReplyMode, prepare, prepared_request};
use crate::azure_dispatch::error::{AzureProviderError, MISSING_EMBEDDING_BASE, MISSING_TEXT_BASE};
use crate::azure_dispatch::fixtures::execution::dispatch;
use crate::azure_dispatch::fixtures::types::{AzureResult, RecordedRequest};
use crate::azure_dispatch::reply;
use crate::azure_dispatch::resolve::{DEFAULT_API_TYPE, merge_defaults, resolve};
use crate::openai_dispatch::call as openai_call;
use crate::openai_dispatch::compose::chat_plan;
use crate::operation::{ChatCompletion, OperationResult};
use crate::prepared::{PreparedRequest, Upstream};

use super::support::{self, Fixture};

const AZURE_BASE: &str = "https://resource.openai.azure.com";

fn base_credentials() -> Credentials {
    Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        api_version: Some(String::from("2024-10-21")),
        api_key: Some(support::key("call-key")),
        ..support::credentials()
    }
}

fn text_call(optional: Object, credentials: Credentials, acompletion: bool) -> TextCall {
    TextCall {
        model: String::from("gpt-3.5-turbo-instruct"),
        messages: support::messages(),
        optional,
        credentials,
        acompletion,
    }
}

fn base_only() -> Credentials {
    Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        ..support::credentials()
    }
}

fn extra_credentials(body: &str) -> Credentials {
    Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        extra_body: Some(support::json(body)),
        ..support::credentials()
    }
}

// Chat composes the deployment URL and auth, the caller's token wins as the SDK's does, and the reply is projected to the chat-shaped view.
pub(super) async fn chat_dispatch() {
    let composition = support::composition(&[]);
    let mut credentials = base_credentials();
    credentials.azure_ad_token = Some(support::key("param-token"));
    credentials.extra_body = Some(support::json(
        r#"{"x-extra":1,"azure_ad_token":"body-token"}"#,
    ));
    credentials.max_retries = Some(support::json("4"));
    let answer = Fixture::ok(support::CHAT_REPLY);
    let optional = support::object(r#"{"temperature":1}"#);
    let call = support::chat("gpt-4o", optional, credentials);
    let result = dispatch(&composition, Request::Chat(Box::new(call)), &answer);
    chat_request_shape(&answer, &result);
    chat_stream_and_supplied(&composition);
}

fn chat_request_shape(answer: &Fixture, result: &AzureResult) {
    let sent = answer.sent();
    assert_eq!(sent.len(), 1);
    let request = &sent[0];
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.url,
        format!("{AZURE_BASE}/openai/deployments/gpt-4o/chat/completions?api-version=2024-10-21")
    );
    assert_eq!(
        support::header(request, "authorization"),
        Some("Bearer param-token")
    );
    assert!(support::header(request, "api-key").is_none());
    assert_eq!(support::header(request, "accept"), Some("application/json"));
    assert_eq!(
        support::header(request, "content-type"),
        Some("application/json")
    );
    assert_eq!(support::body_text(request, "model"), Some("gpt-4o"));
    assert_eq!(support::body_number(request, "temperature"), Some(1));
    assert_eq!(support::body_number(request, "x-extra"), Some(1));
    assert!(support::body_at(request, "azure_ad_token").is_none());
    assert!(support::body_at(request, "max_retries").is_none());
    assert_eq!(result.error, Json::Null);
    assert_eq!(support::reply_text(result, "id"), Some("chatcmpl-1"));
    assert_eq!(
        support::reply_text(result, "object"),
        Some("chat.completion")
    );
    assert_eq!(
        support::reply_text(result, "model"),
        Some("gpt-4o-2024-08-06")
    );
    assert_eq!(support::reply_text(result, "content"), Some("hi"));
    assert_eq!(support::reply_text(result, "finish_reason"), Some("stop"));
    assert_eq!(support::number_at(&result.reply, "total_tokens"), Some(2));
}

// A streamed call marks the body, and the supplied client is rebuilt only for a changed version.
fn chat_stream_and_supplied(composition: &Composition) {
    let mut stream_call = support::chat(
        "gpt-4o",
        Object::new(),
        Credentials {
            api_base: Some(String::from(AZURE_BASE)),
            api_version: Some(String::from("2024-10-21")),
            ..support::credentials()
        },
    );
    stream_call.stream = true;
    stream_call.supplied = Some(SuppliedClient {
        api_version: Some(String::from("2024-10-21")),
    });
    let stream_answer = Fixture::ok(support::CHAT_REPLY);
    dispatch(
        composition,
        Request::Chat(Box::new(stream_call)),
        &stream_answer,
    );
    assert_eq!(
        support::body_at(&stream_answer.sent()[0], "stream"),
        Some(&Json::Bool(true))
    );
    let retained = SuppliedClient {
        api_version: Some(String::from("2024-10-21")),
    };
    assert!(!retained.discarded(Some("2024-10-21")));
    assert!(!retained.discarded(None));
    assert!(retained.discarded(Some("2025-01-01")));
    assert!(SuppliedClient { api_version: None }.discarded(Some("2024-10-21")));
}

// Embeddings keep their own endpoint, key rank and SDK defaults: no chat token pop, no config merge.
pub(super) async fn embedding_dispatch() {
    let mut composition = support::composition(&[
        ("AZURE_API_KEY", "env-azure-key"),
        ("AZURE_OPENAI_API_KEY", "env-openai-key"),
    ]);
    composition.chat_defaults = support::azure_defaults(r#"{"temperature":1}"#);
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        api_version: Some(String::from("2024-10-21")),
        extra_body: Some(support::json(
            r#"{"x-extra":1,"azure_ad_token":"body-token"}"#,
        )),
        ..support::credentials()
    };
    let answer = Fixture::ok(support::EMBEDDING_REPLY);
    let request = support::embedding(
        "text-embedding-3-small",
        r#"{"input":["hi"],"dimensions":2}"#,
        credentials,
    );
    let result = dispatch(&composition, Request::Embedding(Box::new(request)), &answer);
    let sent = answer.sent();
    assert_eq!(sent.len(), 1);
    embedding_request_shape(&sent[0]);
    embedding_reply(&result);
}

fn embedding_request_shape(request: &RecordedRequest) {
    assert_eq!(
        request.url,
        format!(
            "{AZURE_BASE}/openai/deployments/text-embedding-3-small/embeddings?api-version=2024-10-21"
        )
    );
    assert_eq!(support::header(request, "api-key"), Some("env-azure-key"));
    assert_eq!(
        support::body_text(request, "model"),
        Some("text-embedding-3-small")
    );
    let first_input = support::body_at(request, "input").and_then(|input| support::item(input, 0));
    assert_eq!(first_input.and_then(Json::as_str), Some("hi"));
    assert_eq!(support::body_number(request, "dimensions"), Some(2));
    assert_eq!(
        support::body_text(request, "encoding_format"),
        Some("base64")
    );
    assert_eq!(support::body_number(request, "x-extra"), Some(1));
    assert_eq!(
        support::body_text(request, "azure_ad_token"),
        Some("body-token")
    );
    assert!(support::body_at(request, "temperature").is_none());
}

fn embedding_reply(result: &AzureResult) {
    assert_eq!(result.error, Json::Null);
    assert_eq!(support::reply_text(result, "object"), Some("list"));
    assert_eq!(
        support::reply_text(result, "model"),
        Some("text-embedding-3-small")
    );
    assert_eq!(
        support::at(&result.reply, "embedding"),
        Some(&support::json("[0.25,0.5]"))
    );
    assert_eq!(support::number_at(&result.reply, "total_tokens"), Some(2));
}

// Text reads the environment, merges the Azure config defaults, never sends headers or max_retries, and applies the SDK default.
pub(super) async fn text_dispatch() {
    let mut composition = support::composition(&[
        ("AZURE_API_BASE", AZURE_BASE),
        ("AZURE_API_KEY", "env-key"),
        ("AZURE_API_BASE_IGNORED", "x"),
    ]);
    composition.chat_defaults = support::azure_defaults(r#"{"temperature":1,"top_p":1}"#);
    let credentials = Credentials {
        extra_headers: Some(support::headers(&[("x-call", "1")])),
        ..support::credentials()
    };
    let answer = Fixture::ok(support::TEXT_REPLY);
    let call = text_call(Object::new(), credentials, false);
    let result = dispatch(&composition, Request::Text(Box::new(call)), &answer);
    text_request_shape(&answer, &result);
    text_post_call(&composition);
}

fn text_request_shape(answer: &Fixture, result: &AzureResult) {
    let sent = answer.sent();
    assert_eq!(sent.len(), 1);
    let request = &sent[0];
    assert_eq!(
        request.url,
        format!(
            "{AZURE_BASE}/openai/deployments/gpt-3.5-turbo-instruct/completions?api-version=2025-02-01-preview"
        )
    );
    assert_eq!(support::header(request, "api-key"), Some("env-key"));
    assert_eq!(support::header(request, "x-call"), Some("1"));
    assert_eq!(
        support::body_text(request, "model"),
        Some("gpt-3.5-turbo-instruct")
    );
    assert_eq!(support::body_text(request, "prompt"), Some("hi"));
    assert_eq!(support::body_number(request, "temperature"), Some(1));
    assert_eq!(support::body_number(request, "top_p"), Some(1));
    assert!(support::body_at(request, "max_retries").is_none());
    assert_text_reply(result);
}

fn assert_text_reply(result: &AzureResult) {
    assert_eq!(result.error, Json::Null);
    assert_eq!(support::reply_text(result, "id"), Some("cmpl-1"));
    assert_eq!(
        support::reply_text(result, "object"),
        Some("chat.completion")
    );
    assert_eq!(
        support::reply_text(result, "model"),
        Some("gpt-3.5-turbo-instruct")
    );
    assert_eq!(support::reply_text(result, "content"), Some("hello"));
    assert_eq!(support::reply_text(result, "finish_reason"), Some("stop"));
    assert_eq!(support::number_at(&result.reply, "total_tokens"), Some(2));
}

// The post_call condition: acompletion, or a truthy stream.
fn text_post_call(composition: &Composition) {
    let completion_call = text_call(Object::new(), base_only(), true);
    let completion =
        prepare(composition, Request::Text(Box::new(completion_call))).expect("the text call");
    assert!(matches!(
        completion.mode,
        ReplyMode::Text { post_call: true }
    ));
    let optional = support::object(r#"{"stream":true}"#);
    let stream_call = text_call(optional, base_only(), false);
    let streamed =
        prepare(composition, Request::Text(Box::new(stream_call))).expect("the text call");
    assert!(matches!(streamed.mode, ReplyMode::Text { post_call: true }));
    let plain_call = text_call(Object::new(), base_only(), false);
    let plain = prepare(composition, Request::Text(Box::new(plain_call))).expect("the text call");
    assert!(matches!(plain.mode, ReplyMode::Text { post_call: false }));
}

// An Azure call and an OpenAI call keep their own bases, versions, keys and headers, and the Azure token removal leaves OpenAI alone.
pub(super) async fn openai_regression() {
    let azure_composition = support::composition(&[]);
    let credentials = extra_credentials(r#"{"azure_ad_token":"body-token","x-extra":1}"#);
    let azure_answer = Fixture::ok(support::CHAT_REPLY);
    let call = support::chat("gpt-4o", Object::new(), credentials);
    dispatch(
        &azure_composition,
        Request::Chat(Box::new(call)),
        &azure_answer,
    );
    let azure_sent = azure_answer.sent();
    let azure_request = &azure_sent[0];
    assert_eq!(support::header(azure_request, "api-key"), Some("call-key"));
    assert!(support::body_at(azure_request, "azure_ad_token").is_none());
    openai_side();

    // the Azure resolution stays per call: the next Azure call composes the same request again
    let repeat_answer = Fixture::ok(support::CHAT_REPLY);
    let call = support::chat("gpt-4o", Object::new(), base_credentials());
    dispatch(
        &azure_composition,
        Request::Chat(Box::new(call)),
        &repeat_answer,
    );
    assert_eq!(repeat_answer.sent()[0].url, azure_request.url);
    assert_eq!(
        support::header(&repeat_answer.sent()[0], "api-key"),
        Some("call-key")
    );
}

fn openai_composition() -> openai_call::Composition {
    openai_call::Composition {
        credentials: OpenAiCredentialSettings::default(),
        provider: OpenAiProviderSettings::default(),
        globals: LiteLlmGlobals::default(),
        globals_headers: None,
        env_api_key: None,
        chat_defaults: OpenAiSettings::default(),
        text_defaults: TextCompletionSettings::default(),
        catalog: CatalogCapabilities {
            chat_models: BTreeSet::new(),
            text_models: BTreeSet::new(),
        },
    }
}

fn openai_side() {
    let composition = openai_composition();
    let openai_request =
        ChatRequest::parse(br#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#)
            .expect("a chat request");
    let call = openai_call::ChatCall {
        model: String::from("gpt-4o"),
        request: openai_request,
        credentials: openai_call::Credentials {
            api_base: Some(String::from("https://openai.test/v1")),
            api_key: Some(support::key("openai-key")),
            organization: Some(String::from("org-openai")),
        },
        headers: None,
        extra_headers: None,
        timeout: None,
        metadata: None,
    };
    let plan = chat_plan(&composition, &call).expect("the OpenAI chat plan");
    let PreparedRequest::Chat(prepared) = plan.prepared else {
        panic!("an OpenAI chat request");
    };
    assert_eq!(
        prepared.http.url.as_str(),
        "https://openai.test/v1/chat/completions"
    );
    let authorization = prepared
        .http
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    assert_eq!(authorization, Some("Bearer openai-key"));
    let openai_body = support::parse(&prepared.http.body);
    assert!(support::at(&openai_body, "azure_ad_token").is_none());
}

// Only text owns a missing-base error, and an empty environment base passes that guard and fails while the URL is composed instead.
pub(super) async fn missing_base() {
    let composition = support::composition(&[]);
    text_missing_base(&composition);
    embedding_message();
    empty_base_guard();
    chat_missing_base(&composition);
    embedding_missing_base(&composition);
}

fn text_missing_base(composition: &Composition) {
    let answer = Fixture::ok(support::TEXT_REPLY);
    let call = text_call(Object::new(), support::credentials(), false);
    let result = dispatch(composition, Request::Text(Box::new(call)), &answer);
    assert!(answer.sent().is_empty());
    assert_eq!(result.reply, Json::Null);
    assert_eq!(support::error_status(&result), Some(500));
    let call = text_call(Object::new(), support::credentials(), false);
    let error = prepare(composition, Request::Text(Box::new(call)))
        .err()
        .expect("the missing base");
    assert_eq!(error.to_string(), MISSING_TEXT_BASE);
    assert!(matches!(error, AzureProviderError::MissingTextApiBase));
}

fn embedding_message() {
    assert_eq!(
        AzureProviderError::MissingEmbeddingApiBase.to_string(),
        MISSING_EMBEDDING_BASE
    );
}

// an empty environment value is not None: the guard passes and the URL composition fails
fn empty_base_guard() {
    let empty_base = support::composition(&[("AZURE_API_BASE", "")]);
    let call = text_call(Object::new(), support::credentials(), false);
    let error = prepare(&empty_base, Request::Text(Box::new(call)))
        .err()
        .expect("the empty base");
    assert!(matches!(error, AzureProviderError::Url { .. }));
}

// chat has no owned missing-base error: the failure comes from the URL builder
fn chat_missing_base(composition: &Composition) {
    let answer = Fixture::ok(support::CHAT_REPLY);
    let call = support::chat("gpt-4o", Object::new(), support::credentials());
    let result = dispatch(composition, Request::Chat(Box::new(call)), &answer);
    assert!(answer.sent().is_empty());
    assert_eq!(support::error_status(&result), Some(500));
    let call = support::chat("gpt-4o", Object::new(), support::credentials());
    let error = prepare(composition, Request::Chat(Box::new(call)))
        .err()
        .expect("the missing chat base");
    assert!(matches!(error, AzureProviderError::Url { .. }));
}

fn embedding_missing_base(composition: &Composition) {
    let request = support::embedding(
        "text-embedding-3-small",
        r#"{"input":["hi"]}"#,
        support::credentials(),
    );
    let answer = Fixture::ok(support::EMBEDDING_REPLY);
    let result = dispatch(composition, Request::Embedding(Box::new(request)), &answer);
    assert!(answer.sent().is_empty());
    assert_eq!(support::error_status(&result), Some(500));
    let request = support::embedding(
        "text-embedding-3-small",
        r#"{"input":["hi"]}"#,
        support::credentials(),
    );
    let error = prepare(composition, Request::Embedding(Box::new(request)))
        .err()
        .expect("the missing embedding base");
    assert_eq!(error.to_string(), MISSING_EMBEDDING_BASE);
}

fn ranks_composition() -> Composition {
    let environment = [
        ("AZURE_API_BASE", "env-base"),
        ("AZURE_API_VERSION", "env-version"),
        ("AZURE_OPENAI_API_KEY", "env-openai-key"),
        ("AZURE_API_KEY", "env-key"),
        ("AZURE_AD_TOKEN", "env-token"),
        ("AZURE_API_TYPE", "custom"),
    ];
    let mut composition = support::composition(&environment);
    composition.globals = Globals {
        api_base: Some(String::from("global-base")),
        api_version: Some(String::from("global-version")),
        api_key: Some(support::key("global-key")),
        azure_key: Some(support::key("azure-key")),
        headers: None,
        azure_default_api_version: None,
    };
    composition
}

// Every rank main.py applies, the token pop with its mutation, and the malformed-body failures.
pub(super) async fn config_ranks() {
    let mut composition = ranks_composition();
    explicit_ranks(&composition);
    global_ranks(&composition);
    key_ranks(&mut composition);
    empty_values_fall_through();
    version_default();
    token_pop();
    malformed_extra_body();
    keyless_call();
}

fn explicit_ranks(composition: &Composition) {
    let credentials = Credentials {
        api_base: Some(String::from("explicit-base")),
        api_version: Some(String::from("explicit-version")),
        api_key: Some(support::key("explicit-key")),
        extra_body: Some(support::json(
            r#"{"azure_ad_token":"body-token","x-extra":1}"#,
        )),
        ..support::credentials()
    };
    let resolved =
        resolve(composition, credentials, Operation::Chat, Some("gpt-4o")).expect("the chat ranks");
    assert_eq!(resolved.api_base.as_deref(), Some("explicit-base"));
    assert_eq!(resolved.api_version.as_deref(), Some("explicit-version"));
    assert_eq!(
        resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("explicit-key")
    );
    assert_eq!(resolved.api_type, "custom");
    assert_eq!(
        resolved
            .azure_ad_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("body-token")
    );
    assert!(resolved.auth_token.is_none());
    assert_eq!(resolved.extra_body.len(), 1);
    assert_eq!(resolved.extra_body["x-extra"], support::json("1"));
}

fn global_ranks(composition: &Composition) {
    let resolved = resolve(
        composition,
        support::credentials(),
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the global ranks");
    assert_eq!(resolved.api_base.as_deref(), Some("global-base"));
    assert_eq!(resolved.api_version.as_deref(), Some("global-version"));
    assert_eq!(
        resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("global-key")
    );
    assert_eq!(
        resolved
            .azure_ad_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("env-token")
    );
}

// azure_key beats the environment keys, and AZURE_OPENAI_API_KEY beats AZURE_API_KEY
fn key_ranks(composition: &mut Composition) {
    composition.globals.api_key = None;
    let azure_resolved = resolve(
        composition,
        support::credentials(),
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the azure key");
    assert_eq!(
        azure_resolved
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("azure-key")
    );
    composition.globals.azure_key = None;
    let env_resolved = resolve(
        composition,
        support::credentials(),
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the env keys");
    assert_eq!(
        env_resolved
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("env-openai-key")
    );
    let only_azure = support::composition(&[("AZURE_API_KEY", "only-azure-key")]);
    let only_resolved = resolve(
        &only_azure,
        support::credentials(),
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the azure key");
    assert_eq!(
        only_resolved
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("only-azure-key")
    );
    assert_eq!(DEFAULT_API_TYPE, "azure");
}

// an empty explicit value is not a rank: the environment fills it
fn empty_values_fall_through() {
    let environment = support::composition(&[
        ("AZURE_API_BASE", "env-base"),
        ("AZURE_API_VERSION", "env-version"),
    ]);
    let credentials = Credentials {
        api_base: Some(String::new()),
        api_version: Some(String::new()),
        ..support::credentials()
    };
    let resolved =
        resolve(&environment, credentials, Operation::Chat, Some("gpt-4o")).expect("the env ranks");
    assert_eq!(resolved.api_base.as_deref(), Some("env-base"));
    assert_eq!(resolved.api_version.as_deref(), Some("env-version"));
}

// chat alone falls back to the api-version default
fn version_default() {
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        ..support::credentials()
    };
    let resolved = resolve(
        &support::composition(&[]),
        credentials,
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the default version");
    assert_eq!(resolved.api_version.as_deref(), Some("2025-02-01-preview"));
    assert_eq!(resolved.api_type, "azure");
}

// the token pop mutates the body even when it falls back to the environment
fn token_pop() {
    let environment = support::composition(&[("AZURE_AD_TOKEN", "env-token")]);
    let credentials = extra_credentials(r#"{"azure_ad_token":"body-token"}"#);
    let resolved =
        resolve(&environment, credentials, Operation::Chat, Some("gpt-4o")).expect("the token pop");
    assert!(resolved.extra_body.is_empty());
    assert_eq!(
        resolved
            .azure_ad_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("body-token")
    );
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        ..support::credentials()
    };
    let resolved = resolve(&environment, credentials, Operation::Chat, Some("gpt-4o"))
        .expect("the environment token");
    assert_eq!(
        resolved
            .azure_ad_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("env-token")
    );
}

fn malformed_extra_body() {
    let not_object = resolve(
        &support::composition(&[]),
        extra_credentials(r#""text""#),
        Operation::Chat,
        Some("gpt-4o"),
    )
    .err()
    .expect("a non-object extra body");
    assert!(matches!(
        not_object,
        AzureProviderError::ExtraBodyNotAnObject
    ));
    let not_text = resolve(
        &support::composition(&[]),
        extra_credentials(r#"{"azure_ad_token":5}"#),
        Operation::Chat,
        Some("gpt-4o"),
    )
    .err()
    .expect("a non-string token");
    assert!(matches!(not_text, AzureProviderError::ExtraBodyToken));
}

// LiteLLM's SDK refuses a call with no key and no token; the adapter sends it unauthenticated.
fn keyless_call() {
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        ..support::credentials()
    };
    let prepared = prepare(
        &support::composition(&[]),
        Request::Chat(Box::new(support::chat(
            "gpt-4o",
            Object::new(),
            credentials,
        ))),
    )
    .expect("the keyless request");
    assert!(prepared.http.headers.get("authorization").is_none());
    assert!(prepared.http.headers.get("api-key").is_none());
}

// The o-series branch is selected by the detection model and merges the O1 defaults instead.
pub(super) async fn o_series_dispatch() {
    let mut composition = support::composition(&[]);
    composition.chat_defaults = support::azure_defaults(r#"{"temperature":1}"#);
    composition.o_series_defaults = support::o_series_defaults(r#"{"max_tokens":32}"#);
    o_series_model_base(&composition);
    azure_defaults_branch(&composition);
    model_selects_branch(&composition);
    decode_stamps_provider(&composition);
}

fn o_series_model_base(composition: &Composition) {
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        api_version: Some(String::from("2024-10-21")),
        api_key: Some(support::key("call-key")),
        base_model: Some(String::from("o1-preview")),
        ..support::credentials()
    };
    let call = support::chat("deployment-1", Object::new(), credentials);
    let prepared = prepare(composition, Request::Chat(Box::new(call))).expect("the o-series call");
    assert_eq!(
        prepared.http.url.as_str(),
        format!(
            "{AZURE_BASE}/openai/deployments/deployment-1/chat/completions?api-version=2024-10-21"
        )
    );
    let body = support::parse(&prepared.http.body);
    assert_eq!(support::text_at(&body, "model"), Some("deployment-1"));
    assert_eq!(support::number_at(&body, "max_tokens"), Some(32));
    assert!(support::at(&body, "temperature").is_none());
}

// a model the predicate rejects merges the Azure defaults instead
fn azure_defaults_branch(composition: &Composition) {
    let call = support::chat("gpt-4o", Object::new(), base_credentials());
    let prepared = prepare(composition, Request::Chat(Box::new(call))).expect("the azure call");
    let body = support::parse(&prepared.http.body);
    assert_eq!(support::number_at(&body, "temperature"), Some(1));
    assert!(support::at(&body, "max_tokens").is_none());
}

// the model itself selects the branch when no base model is named
fn model_selects_branch(composition: &Composition) {
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        ..support::credentials()
    };
    let call = support::chat("o1-preview", Object::new(), credentials);
    let prepared = prepare(composition, Request::Chat(Box::new(call))).expect("the o-series call");
    let body = support::parse(&prepared.http.body);
    assert_eq!(support::number_at(&body, "max_tokens"), Some(32));
}

// the production decode stamps the reply of an o-series call with the provider name
fn decode_stamps_provider(composition: &Composition) {
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        base_model: Some(String::from("o1-preview")),
        ..support::credentials()
    };
    let call = support::chat("deployment-1", Object::new(), credentials);
    let prepared = prepare(composition, Request::Chat(Box::new(call))).expect("the o-series call");
    let prepared = prepared_request(prepared);
    let response = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: br#"{"id":"chatcmpl-o1","object":"chat.completion","created":1700000000,"model":"o1-preview-2024-12-17","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#.to_vec(),
    };
    let OperationResult::Chat(ChatCompletion::OpenAi(completion)) =
        reply::decode(&prepared, Upstream::Buffered(response)).expect("the o-series reply")
    else {
        panic!("a chat completion");
    };
    assert_eq!(completion.model, "azure/o1-preview-2024-12-17");
}

// Headers merge as completion() merges them, and chat alone inserts max_retries; defaults only fill absent keys.
pub(super) async fn header_retry_merge() {
    let mut composition = support::composition(&[]);
    composition.globals.headers = Some(support::headers(&[("x-global", "g")]));
    header_merge(&composition);
    global_header_fallback(&composition);
    empty_extra_headers(&composition);
    text_headers(&composition);
    retries_acceptance(&composition);
    text_ignores_retries(&composition);
    defaults_fill_absent();
}

fn header_merge(composition: &Composition) {
    let mut credentials = base_credentials();
    credentials.headers = Some(support::headers(&[("x-call", "headers"), ("x-only", "h")]));
    credentials.extra_headers = Some(support::headers(&[("x-call", "extra"), ("x-extra", "e")]));
    let answer = Fixture::ok(support::CHAT_REPLY);
    let call = support::chat("gpt-4o", Object::new(), credentials);
    dispatch(composition, Request::Chat(Box::new(call)), &answer);
    let request = &answer.sent()[0];
    assert_eq!(support::header(request, "x-call"), Some("extra"));
    assert_eq!(support::header(request, "x-only"), Some("h"));
    assert_eq!(support::header(request, "x-extra"), Some("e"));
    assert!(support::header(request, "x-global").is_none());
}

// with no call headers, litellm.headers is the map that reaches the request
fn global_header_fallback(composition: &Composition) {
    let answer = Fixture::ok(support::CHAT_REPLY);
    let call = support::chat("gpt-4o", Object::new(), base_credentials());
    dispatch(composition, Request::Chat(Box::new(call)), &answer);
    assert_eq!(support::header(&answer.sent()[0], "x-global"), Some("g"));
}

// an empty extra_headers map still overwrites the one a body carried
fn empty_extra_headers(composition: &Composition) {
    let mut credentials = base_credentials();
    credentials.extra_headers = Some(support::headers(&[]));
    let answer = Fixture::ok(support::CHAT_REPLY);
    let optional = support::object(r#"{"extra_headers":{"x-body":"1"}}"#);
    let call = support::chat("gpt-4o", optional, credentials);
    dispatch(composition, Request::Chat(Box::new(call)), &answer);
    let sent = answer.sent();
    assert!(support::header(&sent[0], "x-body").is_none());
    assert!(support::body_at(&sent[0], "extra_headers").is_none());
}

// text: only extra_headers reach the request, never headers or litellm.headers
fn text_headers(composition: &Composition) {
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        headers: Some(support::headers(&[("x-call", "headers")])),
        extra_headers: Some(support::headers(&[("x-extra", "e")])),
        ..support::credentials()
    };
    let answer = Fixture::ok(support::TEXT_REPLY);
    let call = text_call(Object::new(), credentials, false);
    dispatch(composition, Request::Text(Box::new(call)), &answer);
    let request = &answer.sent()[0];
    assert_eq!(support::header(request, "x-extra"), Some("e"));
    assert!(support::header(request, "x-call").is_none());
    assert!(support::header(request, "x-global").is_none());
}

// chat accepts 0 and False as max_retries and never sends the key
fn retries_acceptance(composition: &Composition) {
    for retries in [support::json("0"), support::json("false")] {
        let mut credentials = base_credentials();
        credentials.max_retries = Some(retries);
        let answer = Fixture::ok(support::CHAT_REPLY);
        let call = support::chat("gpt-4o", Object::new(), credentials);
        dispatch(composition, Request::Chat(Box::new(call)), &answer);
        assert!(support::body_at(&answer.sent()[0], "max_retries").is_none());
    }
    let mut credentials = base_credentials();
    credentials.max_retries = Some(support::json(r#""many""#));
    let answer = Fixture::ok(support::CHAT_REPLY);
    let call = support::chat("gpt-4o", Object::new(), credentials);
    let result = dispatch(composition, Request::Chat(Box::new(call)), &answer);
    assert_eq!(support::error_status(&result), Some(422));
}

// text ignores max_retries entirely
fn text_ignores_retries(composition: &Composition) {
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        max_retries: Some(support::json(r#""many""#)),
        ..support::credentials()
    };
    let answer = Fixture::ok(support::TEXT_REPLY);
    let call = text_call(Object::new(), credentials, false);
    let result = dispatch(composition, Request::Text(Box::new(call)), &answer);
    assert_eq!(result.error, Json::Null);
    assert!(support::body_at(&answer.sent()[0], "max_retries").is_none());
}

// config defaults fill absent keys only, and an explicit null is a present key
fn defaults_fill_absent() {
    let mut optional = support::object(r#"{"temperature":null}"#);
    let defaults = support::azure_defaults(r#"{"temperature":1,"top_p":1}"#);
    merge_defaults(&mut optional, &defaults).expect("the merged defaults");
    assert_eq!(optional["temperature"], Json::Null);
    assert_eq!(optional["top_p"], support::json("1"));
    let mut composition = support::composition(&[]);
    composition.chat_defaults = support::azure_defaults(r#"{"temperature":1,"top_p":1}"#);
    let call = support::chat(
        "gpt-4o",
        support::object(r#"{"top_p":0}"#),
        base_credentials(),
    );
    let prepared = prepare(&composition, Request::Chat(Box::new(call))).expect("the defaults");
    let body = support::parse(&prepared.http.body);
    assert_eq!(support::number_at(&body, "temperature"), Some(1));
    assert_eq!(support::number_at(&body, "top_p"), Some(0));
}
