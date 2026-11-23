use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_dispatch::call::{Composition, Credentials, Globals, Operation, Request};
use crate::azure_dispatch::compose::prepare;
use crate::azure_dispatch::error::AzureProviderError;
use crate::azure_dispatch::fixtures::execution::dispatch;
use crate::azure_dispatch::resolve::{DEFAULT_API_TYPE, merge_defaults, resolve};

use super::support::{
    self, AZURE_BASE, Fixture, base_credentials, base_only, extra_credentials, text_call,
};

// Only text owns a missing-base error, and an empty environment base passes that guard and fails while the URL is composed instead.
pub(super) fn missing_base() {
    let composition = support::composition(&[]);
    text_missing_base(&composition);
    embedding_message();
    empty_base_guard();
    chat_missing_base(&composition);
    embedding_missing_base(&composition);
}

#[derive(Clone, Copy)]
enum MissingBase {
    Text,
    Chat,
    Embedding,
}

fn missing_base_request(kind: MissingBase) -> Request {
    match kind {
        MissingBase::Text => Request::Text(Box::new(text_call(
            Object::new(),
            support::credentials(),
            false,
        ))),
        MissingBase::Chat => Request::Chat(Box::new(support::chat(
            "gpt-4o",
            Object::new(),
            support::credentials(),
        ))),
        MissingBase::Embedding => Request::Embedding(Box::new(support::embedding(
            "text-embedding-3-small",
            r#"{"input":["hi"]}"#,
            support::credentials(),
        ))),
    }
}

fn missing_base_case(composition: &Composition, kind: MissingBase) -> AzureProviderError {
    let answer = Fixture::ok(match kind {
        MissingBase::Text => support::TEXT_REPLY,
        MissingBase::Chat => support::CHAT_REPLY,
        MissingBase::Embedding => support::EMBEDDING_REPLY,
    });
    let result = dispatch(composition, missing_base_request(kind), &answer);
    assert!(answer.sent().is_empty());
    assert_eq!(result.reply, Json::Null);
    assert_eq!(support::error_status(&result), Some(500));
    prepare(composition, missing_base_request(kind))
        .err()
        .expect("the missing base")
}

fn text_missing_base(composition: &Composition) {
    let error = missing_base_case(composition, MissingBase::Text);
    assert_eq!(
        error.to_string(),
        "api_base is required for Azure OpenAI LLM provider. Either set it dynamically or set the AZURE_API_BASE environment variable."
    );
    assert!(matches!(error, AzureProviderError::MissingTextApiBase));
}

fn embedding_message() {
    assert_eq!(
        AzureProviderError::MissingEmbeddingApiBase.to_string(),
        "No API Base provided for Azure OpenAI LLM provider. Set 'AZURE_API_BASE' in .env"
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
    let error = missing_base_case(composition, MissingBase::Chat);
    assert!(matches!(error, AzureProviderError::Url { .. }));
}

fn embedding_missing_base(composition: &Composition) {
    let error = missing_base_case(composition, MissingBase::Embedding);
    assert_eq!(
        error.to_string(),
        "No API Base provided for Azure OpenAI LLM provider. Set 'AZURE_API_BASE' in .env"
    );
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
pub(super) fn config_ranks() {
    let mut composition = ranks_composition();
    explicit_ranks(&composition);
    global_ranks(&composition);
    key_ranks(&mut composition);
    empty_values_fall_through();
    version_default();
    token_pop();
    extra_body_token_edges();
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
        resolved.api_key.as_ref().map(SecretString::expose_secret),
        Some("explicit-key")
    );
    assert_eq!(resolved.api_type, "custom");
    assert_eq!(
        resolved
            .azure_ad_token
            .as_ref()
            .map(SecretString::expose_secret),
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
        resolved.api_key.as_ref().map(SecretString::expose_secret),
        Some("global-key")
    );
    assert_eq!(
        resolved
            .azure_ad_token
            .as_ref()
            .map(SecretString::expose_secret),
        Some("env-token")
    );
    let mut empty_global = support::composition(&[("AZURE_API_BASE", "env-base")]);
    empty_global.globals.api_base = Some(String::new());
    let fell_through = resolve(
        &empty_global,
        support::credentials(),
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the empty global base");
    assert_eq!(fell_through.api_base.as_deref(), Some("env-base"));
}

// azure_key beats the environment keys, and AZURE_OPENAI_API_KEY beats AZURE_API_KEY
fn key_ranks(composition: &mut Composition) {
    composition.globals.api_key = None;
    assert_key_rank(composition, "azure-key");
    composition.globals.azure_key = None;
    assert_key_rank(composition, "env-openai-key");
    let only_azure = support::composition(&[("AZURE_API_KEY", "only-azure-key")]);
    assert_key_rank(&only_azure, "only-azure-key");
    let empty_environment = support::composition(&[("AZURE_OPENAI_API_KEY", "env-openai-key")]);
    let mut empty_key = base_only();
    empty_key.api_key = Some(support::key(""));
    let keyed = resolve(
        &empty_environment,
        empty_key,
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the empty explicit key");
    assert_eq!(
        keyed.api_key.as_ref().map(SecretString::expose_secret),
        Some("env-openai-key")
    );
    assert_eq!(DEFAULT_API_TYPE, "azure");
}

fn assert_key_rank(composition: &Composition, expected: &str) {
    let resolved = resolve(
        composition,
        support::credentials(),
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the key rank");
    assert_eq!(
        resolved.api_key.as_ref().map(SecretString::expose_secret),
        Some(expected)
    );
}

// an empty explicit value is not a rank and an empty environment secret is unset, so the next rank fills both
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

    let secret_environment = support::composition(&[
        ("AZURE_OPENAI_API_KEY", ""),
        ("AZURE_API_KEY", "fallback-key"),
        ("AZURE_AD_TOKEN", ""),
    ]);
    let secret_resolved = resolve(
        &secret_environment,
        base_only(),
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the empty environment secrets");
    assert_eq!(
        secret_resolved
            .api_key
            .as_ref()
            .map(SecretString::expose_secret),
        Some("fallback-key")
    );
    assert!(secret_resolved.azure_ad_token.is_none());
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
    let empty_version = support::composition(&[("AZURE_API_VERSION", "")]);
    let from_empty = resolve(&empty_version, base_only(), Operation::Chat, Some("gpt-4o"))
        .expect("the empty environment version");
    assert_eq!(
        from_empty.api_version.as_deref(),
        Some("2025-02-01-preview")
    );
    let empty_type = support::composition(&[("AZURE_API_TYPE", "")]);
    let typed = resolve(&empty_type, base_only(), Operation::Chat, Some("gpt-4o"))
        .expect("the empty api type");
    assert_eq!(typed.api_type, "azure");
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
            .map(SecretString::expose_secret),
        Some("body-token")
    );
    let fallback_credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        ..support::credentials()
    };
    let fallback_resolved = resolve(
        &environment,
        fallback_credentials,
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the environment token");
    assert_eq!(
        fallback_resolved
            .azure_ad_token
            .as_ref()
            .map(SecretString::expose_secret),
        Some("env-token")
    );
}

fn extra_body_token_edges() {
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
    let emptied = resolve(
        &support::composition(&[("AZURE_AD_TOKEN", "env-token")]),
        extra_credentials(r#"{"azure_ad_token":""}"#),
        Operation::Chat,
        Some("gpt-4o"),
    )
    .expect("the empty body token");
    assert_eq!(
        emptied
            .azure_ad_token
            .as_ref()
            .map(SecretString::expose_secret),
        Some("env-token")
    );
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

// Headers merge as completion() merges them, and chat alone inserts max_retries; defaults only fill absent keys.
pub(super) fn header_retry_merge() {
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
    let result = dispatch(composition, Request::Chat(Box::new(call)), &answer);
    assert_eq!(result.error, Json::Null);
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
    let result = dispatch(composition, Request::Chat(Box::new(call)), &answer);
    assert_eq!(result.error, Json::Null);
    assert_eq!(support::header(&answer.sent()[0], "x-global"), Some("g"));
}

// an empty extra_headers map still overwrites the one a body carried
fn empty_extra_headers(composition: &Composition) {
    let mut credentials = base_credentials();
    credentials.extra_headers = Some(support::headers(&[]));
    let answer = Fixture::ok(support::CHAT_REPLY);
    let optional = support::object(r#"{"extra_headers":{"x-body":"1"}}"#);
    let call = support::chat("gpt-4o", optional, credentials);
    let result = dispatch(composition, Request::Chat(Box::new(call)), &answer);
    assert_eq!(result.error, Json::Null);
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
    let result = dispatch(composition, Request::Text(Box::new(call)), &answer);
    assert_eq!(result.error, Json::Null);
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
        let result = dispatch(composition, Request::Chat(Box::new(call)), &answer);
        assert_eq!(result.error, Json::Null);
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
