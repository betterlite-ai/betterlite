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

use crate::azure_dispatch::call::{Composition, Credentials, Request, SuppliedClient};
use crate::azure_dispatch::compose::{ReplyMode, prepare, prepared_request};
use crate::azure_dispatch::fixtures::execution::dispatch;
use crate::azure_dispatch::fixtures::types::{AzureResult, RecordedRequest};
use crate::azure_dispatch::reply;
use crate::openai_dispatch::call as openai_call;
use crate::openai_dispatch::compose::chat_plan;
use crate::operation::{ChatCompletion, OperationResult};
use crate::prepared::{PreparedRequest, Upstream};

use super::support::{
    self, AZURE_BASE, Fixture, base_credentials, base_only, extra_credentials, text_call,
};

// Chat composes the deployment URL and auth, the caller's token wins as the SDK's does, and the reply is projected to the chat-shaped view.
pub(super) fn chat_dispatch() {
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
    let buffered =
        prepare(&composition, Request::Chat(Box::new(call.clone()))).expect("the chat call");
    assert!(matches!(
        prepared_request(buffered),
        PreparedRequest::Chat(_)
    ));
    let result = dispatch(&composition, Request::Chat(Box::new(call)), &answer);
    let sent = answer.sent();
    assert_eq!(sent.len(), 1);
    chat_request_line(&sent[0]);
    chat_body_shape(&sent[0]);
    reply_shape(&result, "chatcmpl-1", "gpt-4o-2024-08-06", "hi");
    chat_stream_and_supplied(&composition);
    extra_body_wins_over_stream(&composition);
    deployment_route_base();
    upstream_refusal(&composition);
}

fn chat_request_line(request: &RecordedRequest) {
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
}

fn chat_body_shape(request: &RecordedRequest) {
    assert_eq!(support::body_text(request, "model"), Some("gpt-4o"));
    assert_eq!(support::body_number(request, "temperature"), Some(1));
    assert_eq!(support::body_number(request, "x-extra"), Some(1));
    assert!(support::body_at(request, "azure_ad_token").is_none());
    assert!(support::body_at(request, "max_retries").is_none());
    // a call that never passed stream sends no key at all
    assert!(support::body_at(request, "stream").is_none());
}

fn reply_shape(result: &AzureResult, id: &str, model: &str, content: &str) {
    assert_eq!(result.error, Json::Null);
    assert_eq!(support::reply_text(result, "id"), Some(id));
    assert_eq!(
        support::reply_text(result, "object"),
        Some("chat.completion")
    );
    assert_eq!(support::reply_text(result, "model"), Some(model));
    assert_eq!(support::reply_text(result, "content"), Some(content));
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
    stream_call.stream = Some(true);
    stream_call.supplied = Some(SuppliedClient {
        api_version: Some(String::from("2024-10-21")),
    });
    let streamed = prepare(composition, Request::Chat(Box::new(stream_call.clone())))
        .expect("the streaming call");
    assert!(matches!(
        prepared_request(streamed),
        PreparedRequest::Stream(_)
    ));
    let stream_answer = Fixture::ok(support::CHAT_REPLY);
    let mut mismatched = stream_call.clone();
    mismatched.supplied = Some(SuppliedClient {
        api_version: Some(String::from("2025-01-01")),
    });
    let stream_result = dispatch(
        composition,
        Request::Chat(Box::new(stream_call)),
        &stream_answer,
    );
    assert_eq!(stream_result.error, Json::Null);
    assert_eq!(
        support::body_at(&stream_answer.sent()[0], "stream"),
        Some(&Json::Bool(true))
    );
    let rebuilt = dispatch(
        composition,
        Request::Chat(Box::new(mismatched)),
        &stream_answer,
    );
    assert_eq!(rebuilt.error, Json::Null);
}

// The SDK merges extra_body over the body it was handed, so its stream key wins over the typed argument.
fn extra_body_wins_over_stream(composition: &Composition) {
    let credentials = Credentials {
        extra_body: Some(support::json(r#"{"stream":true}"#)),
        ..base_credentials()
    };
    let mut call = support::chat("gpt-4o", Object::new(), credentials);
    call.stream = Some(false);
    let answer = Fixture::ok(support::CHAT_REPLY);
    let result = dispatch(composition, Request::Chat(Box::new(call)), &answer);
    assert_eq!(result.error, Json::Null);
    assert_eq!(
        support::body_at(&answer.sent()[0], "stream"),
        Some(&Json::Bool(true))
    );
}

// a refusing upstream keeps its status in the observed error, with nothing projected
fn upstream_refusal(composition: &Composition) {
    let answer = Fixture::new(400, r#"{"error":{"message":"refused"}}"#);
    let call = support::chat("gpt-4o", Object::new(), base_credentials());
    let result = dispatch(composition, Request::Chat(Box::new(call)), &answer);
    assert_eq!(support::error_status(&result), Some(400));
    assert_eq!(result.reply, Json::Null);
}

// a base that already ends in the chat route is passed through instead of gaining a deployment prefix
fn deployment_route_base() {
    let credentials = Credentials {
        api_base: Some(String::from("https://proxy.test/chat/completions")),
        api_version: Some(String::from("2024-10-21")),
        api_key: Some(support::key("call-key")),
        ..support::credentials()
    };
    let call = support::chat("gpt-4o", Object::new(), credentials);
    let prepared = prepare(&support::composition(&[]), Request::Chat(Box::new(call)))
        .expect("the deployment route base");
    assert_eq!(
        prepared.http.url.as_str(),
        "https://proxy.test/chat/completions?api-version=2024-10-21"
    );
}

// Embeddings keep their own endpoint, key rank and SDK defaults: no chat token pop, no config merge.
pub(super) fn embedding_dispatch() {
    let composition = embedding_composition();
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        api_version: Some(String::from("2024-10-21")),
        extra_headers: Some(support::headers(&[("x-embed", "e")])),
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

// the embedding case ranks the Azure key from the environment and names a chat default it must not merge
fn embedding_composition() -> Composition {
    let mut composition = support::composition(&[
        ("AZURE_API_KEY", "env-azure-key"),
        ("AZURE_OPENAI_API_KEY", "env-openai-key"),
    ]);
    composition.chat_defaults = support::azure_defaults(r#"{"temperature":1}"#);
    composition
}

fn embedding_request_shape(request: &RecordedRequest) {
    assert_eq!(
        request.url,
        format!(
            "{AZURE_BASE}/openai/deployments/text-embedding-3-small/embeddings?api-version=2024-10-21"
        )
    );
    assert_eq!(support::header(request, "api-key"), Some("env-azure-key"));
    assert_eq!(support::header(request, "x-embed"), Some("e"));
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
pub(super) fn text_dispatch() {
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
    text_token_auth();
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
    reply_shape(result, "cmpl-1", "gpt-3.5-turbo-instruct", "hello");
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

// LiteLLM's SDK call refuses a text azure_ad_token kwarg with a 500; the adapter sends it as the bearer token.
fn text_token_auth() {
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        api_version: Some(String::from("2024-10-21")),
        api_key: Some(support::key("call-key")),
        azure_ad_token: Some(support::key("call-token")),
        ..support::credentials()
    };
    let answer = Fixture::ok(support::TEXT_REPLY);
    let call = text_call(Object::new(), credentials, false);
    let result = dispatch(
        &support::composition(&[]),
        Request::Text(Box::new(call)),
        &answer,
    );
    assert_eq!(result.error, Json::Null);
    let sent = answer.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        support::header(&sent[0], "authorization"),
        Some("Bearer call-token")
    );
    assert!(support::header(&sent[0], "api-key").is_none());
    assert!(support::body_at(&sent[0], "azure_ad_token").is_none());
}

// An Azure call and an OpenAI call keep their own bases, versions, keys and headers, and the Azure token removal leaves OpenAI alone.
pub(super) fn openai_regression() {
    let azure_composition = support::composition(&[]);
    let credentials = extra_credentials(r#"{"azure_ad_token":"body-token","x-extra":1}"#);
    let azure_answer = Fixture::ok(support::CHAT_REPLY);
    let azure_call = support::chat("gpt-4o", Object::new(), credentials);
    let azure_result = dispatch(
        &azure_composition,
        Request::Chat(Box::new(azure_call)),
        &azure_answer,
    );
    assert_eq!(azure_result.error, Json::Null);
    let azure_sent = azure_answer.sent();
    let azure_request = &azure_sent[0];
    assert_eq!(support::header(azure_request, "api-key"), Some("call-key"));
    assert!(support::body_at(azure_request, "azure_ad_token").is_none());
    openai_side();

    // the Azure resolution stays per call: the next Azure call composes the same request again
    let repeat_answer = Fixture::ok(support::CHAT_REPLY);
    let repeat_call = support::chat("gpt-4o", Object::new(), base_credentials());
    let repeat_result = dispatch(
        &azure_composition,
        Request::Chat(Box::new(repeat_call)),
        &repeat_answer,
    );
    assert_eq!(repeat_result.error, Json::Null);
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

// The o-series branch is selected by the detection model and merges the O1 defaults instead.
pub(super) fn o_series_dispatch() {
    let mut composition = support::composition(&[]);
    composition.chat_defaults = support::azure_defaults(r#"{"temperature":1}"#);
    composition.o_series_defaults = support::o_series_defaults(r#"{"max_tokens":32}"#);
    o_series_model_base(&composition);
    azure_defaults_branch(&composition);
    model_selects_branch(&composition);
    decode_stamps_client_model(&composition);
    o_series_fixture_stamp(&composition);
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

// the production decode puts the client's own model back, as the proxy does for every provider
fn decode_stamps_client_model(composition: &Composition) {
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        base_model: Some(String::from("o1-preview")),
        ..support::credentials()
    };
    let mut call = support::chat("deployment-1", Object::new(), credentials);
    call.client_model = Some(String::from("o1-group"));
    let composed = prepare(composition, Request::Chat(Box::new(call))).expect("the o-series call");
    let request = prepared_request(composed);
    let response = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: br#"{"id":"chatcmpl-o1","object":"chat.completion","created":1700000000,"model":"o1-preview-2024-12-17","choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#.to_vec(),
    };
    let OperationResult::Chat(ChatCompletion::OpenAi(completion)) =
        reply::decode(&request, Upstream::Buffered(response)).expect("the o-series reply")
    else {
        panic!("a chat completion");
    };
    assert_eq!(completion.model, "o1-group");
}

// the fixture dispatch stamps an o-series reply with the provider name, as the SDK path does
fn o_series_fixture_stamp(composition: &Composition) {
    let credentials = Credentials {
        api_base: Some(String::from(AZURE_BASE)),
        base_model: Some(String::from("o1-preview")),
        ..support::credentials()
    };
    let call = support::chat("deployment-1", Object::new(), credentials);
    let answer = Fixture::ok(support::CHAT_REPLY);
    let result = dispatch(composition, Request::Chat(Box::new(call)), &answer);
    assert_eq!(result.error, Json::Null);
    assert_eq!(
        support::reply_text(&result, "model"),
        Some("azure/gpt-4o-2024-08-06")
    );
}
