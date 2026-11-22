use std::collections::BTreeSet;

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialSettings;
use betterlite_config::sections::openai_provider::Settings as OpenAiProviderSettings;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::openai_base_config::{CatalogCapabilities, OpenAiSettings};
use betterlite_openai::text_completion_policy::TextCompletionSettings;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use crate::azure_dispatch::call::{Credentials, Globals, SuppliedClient};
use crate::azure_dispatch::compose::{
    prepare_chat_http, prepare_embedding_http, prepare_text_http,
};
use crate::azure_dispatch::dispatch::{dispatch_chat, dispatch_embedding, dispatch_text};
use crate::azure_dispatch::error::{AzureProviderError, MISSING_EMBEDDING_BASE, MISSING_TEXT_BASE};
use crate::azure_dispatch::resolve::{DEFAULT_API_TYPE, merge_defaults, resolve_chat};
use crate::openai_dispatch::call as openai_call;
use crate::openai_dispatch::compose::chat_plan;

use super::support::{self, Fixture};

const AZURE_BASE: &str = "https://resource.openai.azure.com";

fn base_credentials() -> Credentials {
    Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        api_version: Some(String::from("2024-10-21")),
        api_key: Some(support::key("call-key")),
        ..Credentials::default()
    }
}

fn body_of(request: &Value) -> &Value {
    &request["body"]
}

// Chat composes the deployment URL and auth, the caller's token wins as the SDK's does, and the
// reply is projected to the chat-shaped view.
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
    let result = dispatch_chat(
        &composition,
        support::chat("gpt-4o", optional, credentials),
        &answer,
    );
    let sent = answer.sent();
    assert_eq!(sent.len(), 1);
    let request = &sent[0];
    assert_eq!(request["method"], "POST");
    assert_eq!(
        request["url"],
        format!("{AZURE_BASE}/openai/deployments/gpt-4o/chat/completions?api-version=2024-10-21")
    );
    assert_eq!(request["headers"]["authorization"], "Bearer param-token");
    assert!(request["headers"].get("api-key").is_none());
    assert_eq!(request["headers"]["accept"], "application/json");
    assert_eq!(request["headers"]["content-type"], "application/json");
    assert_eq!(body_of(request)["model"], "gpt-4o");
    assert_eq!(body_of(request)["temperature"], 1);
    assert_eq!(body_of(request)["x-extra"], 1);
    assert!(body_of(request).get("azure_ad_token").is_none());
    assert!(body_of(request).get("max_retries").is_none());
    let value = support::value(&result);
    assert_eq!(value["error"], Value::Null);
    assert_eq!(value["reply"]["id"], "chatcmpl-1");
    assert_eq!(value["reply"]["object"], "chat.completion");
    assert_eq!(value["reply"]["model"], "gpt-4o-2024-08-06");
    assert_eq!(value["reply"]["content"], "hi");
    assert_eq!(value["reply"]["finish_reason"], "stop");
    assert_eq!(value["reply"]["total_tokens"], 2);

    // a streamed call marks the body, and the supplied client is rebuilt only for a changed version
    let mut stream_call = support::chat(
        "gpt-4o",
        Object::new(),
        Credentials {
            api_base: Some(String::from(AZURE_BASE)),
            api_version: Some(String::from("2024-10-21")),
            ..Credentials::default()
        },
    );
    stream_call.stream = true;
    stream_call.supplied = Some(SuppliedClient {
        api_version: Some(String::from("2024-10-21")),
    });
    let stream_answer = Fixture::ok(support::CHAT_REPLY);
    dispatch_chat(&composition, stream_call, &stream_answer);
    assert_eq!(stream_answer.sent()[0]["body"]["stream"], true);
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
        ..Credentials::default()
    };
    let answer = Fixture::ok(support::EMBEDDING_REPLY);
    let request = support::embedding(
        "text-embedding-3-small",
        r#"{"input":["hi"],"dimensions":2}"#,
        credentials,
    );
    let result = dispatch_embedding(&composition, request, &answer);
    let sent = answer.sent();
    assert_eq!(sent.len(), 1);
    let sent_request = &sent[0];
    assert_eq!(
        sent_request["url"],
        format!(
            "{AZURE_BASE}/openai/deployments/text-embedding-3-small/embeddings?api-version=2024-10-21"
        )
    );
    assert_eq!(sent_request["headers"]["api-key"], "env-azure-key");
    assert_eq!(body_of(sent_request)["model"], "text-embedding-3-small");
    assert_eq!(body_of(sent_request)["input"][0], "hi");
    assert_eq!(body_of(sent_request)["dimensions"], 2);
    assert_eq!(body_of(sent_request)["encoding_format"], "base64");
    assert_eq!(body_of(sent_request)["x-extra"], 1);
    assert_eq!(body_of(sent_request)["azure_ad_token"], "body-token");
    assert!(body_of(sent_request).get("temperature").is_none());
    let value = support::value(&result);
    assert_eq!(value["error"], Value::Null);
    assert_eq!(value["reply"]["object"], "list");
    assert_eq!(value["reply"]["model"], "text-embedding-3-small");
    assert_eq!(value["reply"]["embedding"], json!([0.25, 0.5]));
    assert_eq!(value["reply"]["total_tokens"], 2);
}

// Text reads the environment, merges the Azure config defaults, never sends headers or max_retries,
// and applies the SDK's own api-version default before the URL is built.
pub(super) async fn text_dispatch() {
    let mut composition = support::composition(&[
        ("AZURE_API_BASE", AZURE_BASE),
        ("AZURE_API_KEY", "env-key"),
        ("AZURE_API_BASE_IGNORED", "x"),
    ]);
    composition.chat_defaults = support::azure_defaults(r#"{"temperature":1,"top_p":1}"#);
    let credentials = Credentials {
        extra_headers: Some(support::headers(&[("x-call", "1")])),
        ..Credentials::default()
    };
    let answer = Fixture::ok(support::TEXT_REPLY);
    let call = support::text("gpt-3.5-turbo-instruct", Object::new(), credentials);
    let result = dispatch_text(&composition, call, &answer);
    let sent = answer.sent();
    assert_eq!(sent.len(), 1);
    let request = &sent[0];
    assert_eq!(
        request["url"],
        format!(
            "{AZURE_BASE}/openai/deployments/gpt-3.5-turbo-instruct/completions?api-version=2025-02-01-preview"
        )
    );
    assert_eq!(request["headers"]["api-key"], "env-key");
    assert_eq!(request["headers"]["x-call"], "1");
    assert_eq!(body_of(request)["model"], "gpt-3.5-turbo-instruct");
    assert_eq!(body_of(request)["prompt"], "hi");
    assert_eq!(body_of(request)["temperature"], 1);
    assert_eq!(body_of(request)["top_p"], 1);
    assert!(body_of(request).get("max_retries").is_none());
    let value = support::value(&result);
    assert_eq!(value["error"], Value::Null);
    assert_eq!(value["reply"]["id"], "cmpl-1");
    assert_eq!(value["reply"]["object"], "chat.completion");
    assert_eq!(value["reply"]["content"], "hello");
    assert_eq!(value["reply"]["finish_reason"], "stop");
    assert_eq!(value["reply"]["total_tokens"], 2);

    // the post_call condition: acompletion, or a truthy stream
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        ..Credentials::default()
    };
    let mut completion_call = support::text("gpt-3.5-turbo-instruct", Object::new(), credentials);
    completion_call.acompletion = true;
    let (_, post_call) = prepare_text_http(&composition, completion_call).expect("the text call");
    assert!(post_call);
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        ..Credentials::default()
    };
    let stream_call = support::text(
        "gpt-3.5-turbo-instruct",
        support::object(r#"{"stream":true}"#),
        credentials,
    );
    let (_, post_call) = prepare_text_http(&composition, stream_call).expect("the text call");
    assert!(post_call);
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        ..Credentials::default()
    };
    let plain_call = support::text("gpt-3.5-turbo-instruct", Object::new(), credentials);
    let (_, post_call) = prepare_text_http(&composition, plain_call).expect("the text call");
    assert!(!post_call);
}

// An Azure call and an OpenAI call keep their own bases, versions, keys and headers, and the Azure
// token removal leaves the OpenAI configuration alone.
pub(super) async fn openai_regression() {
    let azure_composition = support::composition(&[]);
    let mut credentials = base_credentials();
    credentials.extra_body = Some(support::json(
        r#"{"azure_ad_token":"body-token","x-extra":1}"#,
    ));
    let azure_answer = Fixture::ok(support::CHAT_REPLY);
    dispatch_chat(
        &azure_composition,
        support::chat("gpt-4o", Object::new(), credentials),
        &azure_answer,
    );
    let azure_request = &azure_answer.sent()[0];
    assert_eq!(azure_request["headers"]["api-key"], "call-key");
    assert!(body_of(azure_request).get("azure_ad_token").is_none());

    let openai_composition = openai_call::Composition {
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
    };
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
    let plan = chat_plan(&openai_composition, &call).expect("the OpenAI chat plan");
    let http = plan.prepared.http().expect("a prepared HTTP request");
    assert_eq!(http.url.as_str(), "https://openai.test/v1/chat/completions");
    let authorization = http
        .headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());
    assert_eq!(authorization, Some("Bearer openai-key"));
    let openai_body: Value = serde_json::from_slice(&http.body).expect("the OpenAI body");
    assert!(openai_body.get("azure_ad_token").is_none());

    // the Azure resolution stays per call: the next Azure call composes the same request again
    let repeat_credentials = base_credentials();
    let repeat_answer = Fixture::ok(support::CHAT_REPLY);
    dispatch_chat(
        &azure_composition,
        support::chat("gpt-4o", Object::new(), repeat_credentials),
        &repeat_answer,
    );
    assert_eq!(repeat_answer.sent()[0]["url"], azure_request["url"]);
    assert_eq!(repeat_answer.sent()[0]["headers"]["api-key"], "call-key");
}

// Only text owns a missing-base error, and an empty environment base passes that guard and fails
// while the URL is composed instead.
pub(super) async fn missing_base() {
    let composition = support::composition(&[]);
    let answer = Fixture::ok(support::TEXT_REPLY);
    let call = support::text(
        "gpt-3.5-turbo-instruct",
        Object::new(),
        Credentials::default(),
    );
    let result = dispatch_text(&composition, call, &answer);
    assert!(answer.sent().is_empty());
    let value = support::value(&result);
    assert_eq!(value["reply"], Value::Null);
    assert_eq!(value["error"]["status"], 500);
    let error = prepare_text_http(
        &composition,
        support::text(
            "gpt-3.5-turbo-instruct",
            Object::new(),
            Credentials::default(),
        ),
    )
    .err()
    .expect("the missing base");
    assert_eq!(error.to_string(), MISSING_TEXT_BASE);
    assert!(matches!(error, AzureProviderError::MissingTextApiBase));
    assert_eq!(
        AzureProviderError::MissingEmbeddingApiBase.to_string(),
        MISSING_EMBEDDING_BASE
    );

    // an empty environment value is not None: the guard passes and the URL composition fails
    let empty_base = support::composition(&[("AZURE_API_BASE", "")]);
    let error = prepare_text_http(
        &empty_base,
        support::text(
            "gpt-3.5-turbo-instruct",
            Object::new(),
            Credentials::default(),
        ),
    )
    .err()
    .expect("the empty base");
    assert!(matches!(error, AzureProviderError::Url { .. }));

    // chat has no owned missing-base error: the failure comes from the URL builder
    let chat_answer = Fixture::ok(support::CHAT_REPLY);
    let result = dispatch_chat(
        &composition,
        support::chat("gpt-4o", Object::new(), Credentials::default()),
        &chat_answer,
    );
    assert!(chat_answer.sent().is_empty());
    assert_eq!(support::value(&result)["error"]["status"], 500);
    let error = prepare_chat_http(
        &composition,
        support::chat("gpt-4o", Object::new(), Credentials::default()),
    )
    .err()
    .expect("the missing chat base");
    assert!(matches!(error, AzureProviderError::Url { .. }));

    // embedding owns its own message
    let embedding_answer = Fixture::ok(support::EMBEDDING_REPLY);
    let result = dispatch_embedding(
        &composition,
        support::embedding(
            "text-embedding-3-small",
            r#"{"input":["hi"]}"#,
            Credentials::default(),
        ),
        &embedding_answer,
    );
    assert!(embedding_answer.sent().is_empty());
    assert_eq!(support::value(&result)["error"]["status"], 500);
    let error = prepare_embedding_http(
        &composition,
        support::embedding(
            "text-embedding-3-small",
            r#"{"input":["hi"]}"#,
            Credentials::default(),
        ),
    )
    .err()
    .expect("the missing embedding base");
    assert_eq!(error.to_string(), MISSING_EMBEDDING_BASE);
}

// Every rank main.py applies, the token pop with its mutation, and the malformed-body failures.
pub(super) async fn config_ranks() {
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

    // the call's own values win over every rank below them
    let mut credentials = Credentials {
        api_base: Some(String::from("explicit-base")),
        api_version: Some(String::from("explicit-version")),
        api_key: Some(support::key("explicit-key")),
        extra_body: Some(support::json(
            r#"{"azure_ad_token":"body-token","x-extra":1}"#,
        )),
        ..Credentials::default()
    };
    let resolved = resolve_chat(&composition, credentials, "gpt-4o").expect("the chat ranks");
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

    // the module globals rank next, then the environment
    credentials = Credentials::default();
    let resolved = resolve_chat(&composition, credentials, "gpt-4o").expect("the global ranks");
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

    // azure_key beats the environment keys, and AZURE_OPENAI_API_KEY beats AZURE_API_KEY
    composition.globals.api_key = None;
    let credentials = Credentials::default();
    let resolved = resolve_chat(&composition, credentials, "gpt-4o").expect("the azure key");
    assert_eq!(
        resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("azure-key")
    );
    composition.globals.azure_key = None;
    let credentials = Credentials::default();
    let resolved = resolve_chat(&composition, credentials, "gpt-4o").expect("the env keys");
    assert_eq!(
        resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("env-openai-key")
    );
    let resolved = resolve_chat(
        &support::composition(&[("AZURE_API_KEY", "only-azure-key")]),
        Credentials::default(),
        "gpt-4o",
    )
    .expect("the azure key");
    assert_eq!(
        resolved.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("only-azure-key")
    );
    assert_eq!(DEFAULT_API_TYPE, "azure");

    // chat alone falls back to the api-version default
    let resolved = resolve_chat(
        &support::composition(&[]),
        Credentials {
            api_base: Some(String::from(AZURE_BASE)),
            ..Credentials::default()
        },
        "gpt-4o",
    )
    .expect("the default version");
    assert_eq!(resolved.api_version.as_deref(), Some("2025-02-01-preview"));

    // the token pop mutates the body even when it falls back to the environment
    let resolved = resolve_chat(
        &support::composition(&[("AZURE_AD_TOKEN", "env-token")]),
        Credentials {
            api_base: Some(String::from(AZURE_BASE)),
            extra_body: Some(support::json(r#"{"azure_ad_token":"body-token"}"#)),
            ..Credentials::default()
        },
        "gpt-4o",
    )
    .expect("the token pop");
    assert!(resolved.extra_body.is_empty());
    assert_eq!(
        resolved
            .azure_ad_token
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("body-token")
    );

    // malformed extra bodies fail
    let error = resolve_chat(
        &support::composition(&[]),
        Credentials {
            api_base: Some(String::from(AZURE_BASE)),
            extra_body: Some(support::json(r#""text""#)),
            ..Credentials::default()
        },
        "gpt-4o",
    )
    .err()
    .expect("a non-object extra body");
    assert!(matches!(error, AzureProviderError::ExtraBodyNotAnObject));
    let error = resolve_chat(
        &support::composition(&[]),
        Credentials {
            api_base: Some(String::from(AZURE_BASE)),
            extra_body: Some(support::json(r#"{"azure_ad_token":5}"#)),
            ..Credentials::default()
        },
        "gpt-4o",
    )
    .err()
    .expect("a non-string token");
    assert!(matches!(error, AzureProviderError::ExtraBodyToken));

    // LiteLLM's SDK refuses a call with no key and no token; the adapter sends it unauthenticated.
    let (http, _json_mode) = prepare_chat_http(
        &support::composition(&[]),
        support::chat(
            "gpt-4o",
            Object::new(),
            Credentials {
                api_base: Some(String::from(AZURE_BASE)),
                ..Credentials::default()
            },
        ),
    )
    .expect("the keyless request");
    assert!(http.headers.get("authorization").is_none());
    assert!(http.headers.get("api-key").is_none());
}

// The o-series branch is selected by the detection model and merges the O1 defaults instead.
pub(super) async fn o_series_dispatch() {
    let mut composition = support::composition(&[]);
    composition.chat_defaults = support::azure_defaults(r#"{"temperature":1}"#);
    composition.o_series_defaults = support::o_series_defaults(r#"{"max_tokens":32}"#);
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        api_version: Some(String::from("2024-10-21")),
        api_key: Some(support::key("call-key")),
        base_model: Some(String::from("o1-preview")),
        ..Credentials::default()
    };
    let call = support::chat("deployment-1", Object::new(), credentials);
    let (http, _json_mode) = prepare_chat_http(&composition, call).expect("the o-series call");
    assert_eq!(
        http.url.as_str(),
        format!(
            "{AZURE_BASE}/openai/deployments/deployment-1/chat/completions?api-version=2024-10-21"
        )
    );
    let body: Value = serde_json::from_slice(&http.body).expect("the o-series body");
    assert_eq!(body["model"], "deployment-1");
    assert_eq!(body["max_tokens"], 32);
    assert!(body.get("temperature").is_none());

    // a model the predicate rejects merges the Azure defaults instead
    let credentials = base_credentials();
    let call = support::chat("gpt-4o", Object::new(), credentials);
    let (http, _json_mode) = prepare_chat_http(&composition, call).expect("the azure call");
    let body: Value = serde_json::from_slice(&http.body).expect("the azure body");
    assert_eq!(body["temperature"], 1);
    assert!(body.get("max_tokens").is_none());

    // the model itself selects the branch when no base model is named
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        ..Credentials::default()
    };
    let call = support::chat("o1-preview", Object::new(), credentials);
    let (http, _json_mode) = prepare_chat_http(&composition, call).expect("the o-series call");
    let body: Value = serde_json::from_slice(&http.body).expect("the o-series body");
    assert_eq!(body["max_tokens"], 32);
}

// Headers merge as completion() merges them, and chat alone inserts max_retries; defaults only fill
// keys the caller left out.
pub(super) async fn header_retry_merge() {
    let mut composition = support::composition(&[]);
    composition.globals.headers = Some(support::headers(&[("x-global", "g")]));
    let mut credentials = base_credentials();
    credentials.headers = Some(support::headers(&[("x-call", "headers"), ("x-only", "h")]));
    credentials.extra_headers = Some(support::headers(&[("x-call", "extra"), ("x-extra", "e")]));
    let answer = Fixture::ok(support::CHAT_REPLY);
    dispatch_chat(
        &composition,
        support::chat("gpt-4o", Object::new(), credentials),
        &answer,
    );
    let request = &answer.sent()[0];
    assert_eq!(request["headers"]["x-call"], "extra");
    assert_eq!(request["headers"]["x-only"], "h");
    assert_eq!(request["headers"]["x-extra"], "e");
    assert!(request["headers"].get("x-global").is_none());

    // with no call headers, litellm.headers is the map that reaches the request
    let credentials = base_credentials();
    let answer = Fixture::ok(support::CHAT_REPLY);
    dispatch_chat(
        &composition,
        support::chat("gpt-4o", Object::new(), credentials),
        &answer,
    );
    assert_eq!(answer.sent()[0]["headers"]["x-global"], "g");

    // an empty extra_headers map still overwrites the one a body carried
    let mut credentials = base_credentials();
    credentials.extra_headers = Some(support::headers(&[]));
    let answer = Fixture::ok(support::CHAT_REPLY);
    let optional = support::object(r#"{"extra_headers":{"x-body":"1"}}"#);
    dispatch_chat(
        &composition,
        support::chat("gpt-4o", optional, credentials),
        &answer,
    );
    let sent = answer.sent();
    assert!(sent[0]["headers"].get("x-body").is_none());
    assert!(body_of(&sent[0]).get("extra_headers").is_none());

    // text: only extra_headers reach the request, never headers or litellm.headers
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        headers: Some(support::headers(&[("x-call", "headers")])),
        extra_headers: Some(support::headers(&[("x-extra", "e")])),
        ..Credentials::default()
    };
    let answer = Fixture::ok(support::TEXT_REPLY);
    let call = support::text("gpt-3.5-turbo-instruct", Object::new(), credentials);
    dispatch_text(&composition, call, &answer);
    let request = &answer.sent()[0];
    assert_eq!(request["headers"]["x-extra"], "e");
    assert!(request["headers"].get("x-call").is_none());
    assert!(request["headers"].get("x-global").is_none());

    // chat accepts 0 and False as max_retries and never sends the key; text ignores it entirely
    for retries in [support::json("0"), support::json("false")] {
        let mut credentials = base_credentials();
        credentials.max_retries = Some(retries);
        let answer = Fixture::ok(support::CHAT_REPLY);
        dispatch_chat(
            &composition,
            support::chat("gpt-4o", Object::new(), credentials),
            &answer,
        );
        assert!(body_of(&answer.sent()[0]).get("max_retries").is_none());
    }
    let mut credentials = base_credentials();
    credentials.max_retries = Some(support::json(r#""many""#));
    let answer = Fixture::ok(support::CHAT_REPLY);
    let result = dispatch_chat(
        &composition,
        support::chat("gpt-4o", Object::new(), credentials),
        &answer,
    );
    assert_eq!(support::value(&result)["error"]["status"], 422);
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        max_retries: Some(support::json(r#""many""#)),
        ..Credentials::default()
    };
    let answer = Fixture::ok(support::TEXT_REPLY);
    let call = support::text("gpt-3.5-turbo-instruct", Object::new(), credentials);
    let result = dispatch_text(&composition, call, &answer);
    assert_eq!(support::value(&result)["error"], Value::Null);

    // config defaults fill absent keys only, and an explicit null is a present key
    let mut optional = support::object(r#"{"temperature":null}"#);
    let defaults = support::azure_defaults(r#"{"temperature":1,"top_p":1}"#);
    merge_defaults(&mut optional, &defaults).expect("the merged defaults");
    assert_eq!(optional["temperature"], Json::Null);
    assert_eq!(optional["top_p"], support::json("1"));
    let mut composition = support::composition(&[]);
    composition.chat_defaults = support::azure_defaults(r#"{"temperature":1,"top_p":1}"#);
    let credentials = base_credentials();
    let call = support::chat("gpt-4o", support::object(r#"{"top_p":0}"#), credentials);
    let (http, _json_mode) = prepare_chat_http(&composition, call).expect("the defaults");
    let body: Value = serde_json::from_slice(&http.body).expect("the body");
    assert_eq!(body["temperature"], 1);
    assert_eq!(body["top_p"], 0);
}
