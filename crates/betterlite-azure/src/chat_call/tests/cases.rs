#![cfg_attr(
    test,
    expect(
        clippy::cognitive_complexity,
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        clippy::disallowed_types,
        clippy::field_reassign_with_default,
        clippy::shadow_unrelated,
        clippy::too_many_lines,
        reason = "each case states one LiteLLM behavior, its fixture JSON carries bare numbers and json! literals, the sync and async halves rebind the recorder, reply and error, and each case scripts one harness field at a time"
    )
)]

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Object;
use futures_util::StreamExt as _;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value, json};

use crate::azure_urls::AuthParams;
use crate::chat_call::error::{
    ASYNC_CLIENT, AzureChatError, MAX_RETRIES_TYPE, MISSING_MODEL_OR_MESSAGES, SYNC_CLIENT,
};
use crate::chat_call::handler::ChatOutcome;
use crate::chat_call::o_series::{
    self, OSeriesCall, OSeriesDelegate, OSeriesError, OSeriesForward,
};
use crate::chat_call::response::ChatReply;
use crate::endpoints::projection::{ClientKind, LiteLlmParams};

use super::support::{
    Answer, DEFAULT_BODY, GetRecord, Harness, Recorder, RequestRecord, ScriptedSource,
    ScriptedTransport, block_on, decisions_of, identifier, json_headers, messages_of, status_of,
};

const MODEL: &str = "gpt-4o-deployment";
const OUTPUT_LIMIT_MARKER: &str =
    "could not finish the message because max_tokens or model output limit was reached";

fn buffered(outcome: ChatOutcome) -> Box<ChatReply> {
    match outcome {
        ChatOutcome::Buffered(reply) => reply,
        ChatOutcome::Stream(_stream) => panic!("the owned cases never stream"),
    }
}

fn error_of(result: Result<ChatOutcome, AzureChatError>) -> AzureChatError {
    match result {
        Ok(_outcome) => panic!("the case expects a failure"),
        Err(error) => error,
    }
}

fn request_body(request: &RequestRecord) -> &Map<String, Value> {
    request
        .data
        .as_object()
        .expect("the sent body is a JSON object")
}

fn first_body(source: &ScriptedSource) -> Map<String, Value> {
    let requests = source.requests();
    assert_eq!(requests.len(), 1);
    request_body(&requests[0]).clone()
}

fn text_of(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

pub(super) fn chat_deployment_request() {
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut harness = Harness::default();
    harness.optional_params = json!({"temperature": 0.2, "max_retries": 1, "json_mode": false});
    let mut recorder = Recorder::default();
    let reply =
        block_on(async { buffered(harness.run(&source, &mut recorder).await.expect("reply")) });

    assert_eq!(
        reply.response.choices[0].message.content.as_deref(),
        Some("Hello!")
    );
    assert_eq!(reply.response.id, "chatcmpl-1");
    let body = first_body(&source);
    assert_eq!(body.get("model"), Some(&json!(MODEL)));
    assert_eq!(body.get("temperature"), Some(&json!(0.2)));
    assert!(body.get("max_retries").is_none());
    assert!(body.get("json_mode").is_none());

    let requests = source.requests();
    assert_eq!(requests[0].timeout.to_bits(), 30.0_f64.to_bits());
    let gets = source.gets();
    assert_eq!(gets[0].model.as_deref(), Some(MODEL));
    assert_eq!(gets[0].api_version.as_deref(), Some("2024-02-01"));
    assert!(!gets[0].is_async);

    let pre = &recorder.pre[0];
    assert_eq!(
        pre.input,
        decisions_of(json!([{"role": "user", "content": "hi"}]))
    );
    assert_eq!(
        pre.api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        Some("key".to_owned())
    );
    let headers = pre.additional_args.headers.as_ref().expect("headers");
    assert_eq!(text_of(headers.get("api_key")), Some("key"));
    assert_eq!(headers.get("azure_ad_token"), Some(&Json::Null));
    let Some(RawBody::Object(logged)) = &pre.additional_args.complete_input_dict else {
        panic!("the pre call logs the request body");
    };
    assert_eq!(logged.get("model"), Some(&Json::Text(MODEL.to_owned())));
    assert_eq!(pre.additional_args.openai_sdk, Some(true));
    assert_eq!(
        pre.additional_args.passthrough.get("api_version"),
        Some(&Json::Text("2024-02-01".to_owned()))
    );

    let post = &recorder.post[0];
    assert_eq!(
        post.original_response,
        decisions_of(json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1_700_000_000,
            "model": "gpt-4o",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hello!"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}
        }))
    );
    assert_eq!(
        text_of(
            post.additional_args
                .headers
                .as_ref()
                .and_then(|headers| headers.get("content-type"))
        ),
        Some("application/json")
    );

    let hidden = reply
        .hidden_params
        .get("additional_headers")
        .expect("additional headers");
    let Json::Object(fields) = hidden else {
        panic!("additional headers are an object");
    };
    assert_eq!(
        fields.get("llm_provider-content-type"),
        Some(&Json::Text("application/json".to_owned()))
    );
    assert!(reply.hidden_params.get("headers").is_none());
}

pub(super) fn azure_v1_body_model() {
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut harness = Harness::default();
    harness.api_base = "https://resource.openai.azure.com/openai/v1".to_owned();
    let mut recorder = Recorder::default();
    let reply =
        block_on(async { buffered(harness.run(&source, &mut recorder).await.expect("reply")) });

    assert_eq!(reply.response.model, "gpt-4o");
    let body = first_body(&source);
    assert_eq!(body.get("model"), Some(&json!(MODEL)));
    assert_eq!(body.get("stream"), None);
    assert_eq!(
        recorder.pre[0].additional_args.api_base.as_deref(),
        Some("https://resource.openai.azure.com/openai/v1")
    );
}

pub(super) fn cross_deployment_auth() {
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, None, Answer::reply());
    let mut harness = Harness::default();
    harness.api_key = None;
    harness.azure_ad_token = Some("ad-token".to_owned());
    harness.litellm_params = LiteLlmParams {
        azure_ad_token: Some(SecretString::from("ad-token")),
        auth: AuthParams::default(),
        max_retries: None,
        timeout: None,
    };
    let mut recorder = Recorder::default();
    let _reply =
        block_on(async { buffered(harness.run(&source, &mut recorder).await.expect("reply")) });

    let gets = source.gets();
    assert_eq!(gets[0].api_key, None);
    assert_eq!(gets[0].model.as_deref(), Some(MODEL));
    assert_eq!(
        gets[0]
            .litellm_params
            .azure_ad_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        Some("ad-token".to_owned())
    );
    let headers = recorder.pre[0]
        .additional_args
        .headers
        .as_ref()
        .expect("headers");
    assert_eq!(headers.get("api_key"), Some(&Json::Null));
    assert_eq!(text_of(headers.get("azure_ad_token")), Some("ad-token"));
    assert_eq!(first_body(&source).get("model"), Some(&json!(MODEL)));
}

pub(super) fn missing_model_messages_422() {
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut harness = Harness::default();
    harness.model = None;
    let mut recorder = Recorder::default();
    let error = block_on(async { error_of(harness.run(&source, &mut recorder).await) });
    assert_eq!(status_of(&error), Some(422));
    assert_eq!(error.to_string(), MISSING_MODEL_OR_MESSAGES);
    assert!(source.gets().is_empty());
    assert!(source.requests().is_empty());
    assert!(recorder.pre.is_empty());

    // empty strings and lists are values, not the None that raises
    let empty_source = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut empty = Harness::default();
    empty.model = Some(String::new());
    empty.messages = Some(Vec::new());
    let mut recorder = Recorder::default();
    let result = block_on(async { empty.run(&empty_source, &mut recorder).await });
    assert!(!matches!(
        &result,
        Err(AzureChatError::Azure { status: 422, .. })
    ));
    assert_eq!(empty_source.requests().len(), 1);
}

pub(super) fn max_retries_default_two() {
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut harness = Harness::default();
    harness.api_base = "https://gateway.ai.cloudflare.com/v1/account/gateway".to_owned();
    harness.supplied = false;
    let mut recorder = Recorder::default();
    let _reply =
        block_on(async { buffered(harness.run(&source, &mut recorder).await.expect("reply")) });
    let gateways = source.gateways();
    assert_eq!(gateways.len(), 1);
    assert_eq!(gateways[0].max_retries, json!(2));
    assert_eq!(gateways[0].model, MODEL);
    assert_eq!(
        gateways[0].api_base,
        "https://gateway.ai.cloudflare.com/v1/account/gateway"
    );
    // the gateway's client is the one the later send uses
    assert!(source.gets()[0].supplied);
    assert_eq!(first_body(&source).get("model"), Some(&Value::Null));

    let explicit = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut chosen = Harness::default();
    chosen.api_base = "https://gateway.ai.cloudflare.com/v1/account/gateway".to_owned();
    chosen.supplied = false;
    chosen.optional_params = json!({"max_retries": 5});
    let mut recorder = Recorder::default();
    let _reply =
        block_on(async { buffered(chosen.run(&explicit, &mut recorder).await.expect("reply")) });
    assert_eq!(explicit.gateways()[0].max_retries, json!(5));
}

pub(super) fn invalid_retries_422() {
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut harness = Harness::default();
    harness.optional_params = json!({"max_retries": "two"});
    let mut recorder = Recorder::default();
    let error = block_on(async { error_of(harness.run(&source, &mut recorder).await) });
    assert_eq!(status_of(&error), Some(422));
    assert_eq!(error.to_string(), MAX_RETRIES_TYPE);
    // the sync path logs before it validates
    assert_eq!(recorder.pre.len(), 1);
    assert!(recorder.post.is_empty());
    assert!(source.gets().is_empty());
    assert!(source.requests().is_empty());

    // isinstance(True, int) is true, so a boolean passes the guard
    let boolean = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut accepted = Harness::default();
    accepted.optional_params = json!({"max_retries": true});
    let mut recorder = Recorder::default();
    let _reply =
        block_on(async { buffered(accepted.run(&boolean, &mut recorder).await.expect("reply")) });
    assert_eq!(boolean.requests().len(), 1);

    // a negative int is an int too: only a non-number fails the guard
    let negative = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut signed = Harness::default();
    signed.optional_params = json!({"max_retries": -1});
    let mut recorder = Recorder::default();
    let _reply =
        block_on(async { buffered(signed.run(&negative, &mut recorder).await.expect("reply")) });
    assert_eq!(negative.requests().len(), 1);
}

pub(super) fn cloudflare_model_null() {
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut harness = Harness::default();
    harness.api_base = "https://gateway.ai.cloudflare.com/v1/account/gateway".to_owned();
    harness.optional_params = json!({"temperature": 0.5});
    let mut recorder = Recorder::default();
    let _reply =
        block_on(async { buffered(harness.run(&source, &mut recorder).await.expect("reply")) });

    // with a supplied client the gateway builder hands it back, so nothing is constructed
    assert!(source.gateways().is_empty());
    let body = first_body(&source);
    assert_eq!(body.get("model"), Some(&Value::Null));
    assert_eq!(
        body.get("messages"),
        Some(&json!([{"role": "user", "content": "hi"}]))
    );
    assert_eq!(body.get("temperature"), Some(&json!(0.5)));
}

pub(super) fn gpt5_base_model_selection() {
    // a base model outside the GPT-5 family keeps the base transformation, which never strips the route
    let routed = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut base = Harness::default();
    base.model = Some("gpt5_series/gpt-5".to_owned());
    base.base_model = Some("gpt-4o".to_owned());
    let mut recorder = Recorder::default();
    let _reply =
        block_on(async { buffered(base.run(&routed, &mut recorder).await.expect("reply")) });
    assert_eq!(
        first_body(&routed).get("model"),
        Some(&json!("gpt5_series/gpt-5"))
    );

    // the base model decides: the GPT-5 transformation strips the routing prefix
    let selected = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut gpt5 = Harness::default();
    gpt5.model = Some("gpt5_series/gpt-5".to_owned());
    gpt5.base_model = Some("gpt-5".to_owned());
    let mut recorder = Recorder::default();
    let _reply =
        block_on(async { buffered(gpt5.run(&selected, &mut recorder).await.expect("reply")) });
    assert_eq!(first_body(&selected).get("model"), Some(&json!("gpt-5")));

    // without a base model the deployment name itself selects the transformation
    let direct = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut own = Harness::default();
    own.model = Some("gpt5_series/gpt-5".to_owned());
    let mut recorder = Recorder::default();
    let _reply =
        block_on(async { buffered(own.run(&direct, &mut recorder).await.expect("reply")) });
    assert_eq!(first_body(&direct).get("model"), Some(&json!("gpt-5")));
}

const JSON_MODE_BODY: &str = concat!(
    r#"{"id":"chatcmpl-2","object":"chat.completion","created":1700000001,"model":"gpt-4o","#,
    r#""choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"json_tool_call","arguments":"{\"a\":1}"}}]},"finish_reason":"stop"}],"#,
    r#""usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#
);

pub(super) fn json_mode_projection() {
    let source = ScriptedSource::new(
        ClientKind::AzureOpenAi,
        Some("key"),
        Answer::with_body(JSON_MODE_BODY),
    );
    let mut harness = Harness::default();
    harness.optional_params = json!({"json_mode": true});
    harness.headers = Some(BTreeMap::from([("x-caller".to_owned(), "1".to_owned())]));
    let mut recorder = Recorder::default();
    let reply =
        block_on(async { buffered(harness.run(&source, &mut recorder).await.expect("reply")) });

    let body = first_body(&source);
    assert_eq!(body.get("extra_headers"), Some(&json!({"x-caller": "1"})));
    assert!(body.get("json_mode").is_none());
    let message = &reply.response.choices[0].message;
    assert_eq!(message.content.as_deref(), Some("{\"a\":1}"));
    assert!(message.tool_calls.is_none());
}

pub(super) fn output_limit_recovery() {
    let answer = Answer::BadRequest {
        message: format!("Error code: 400 - {OUTPUT_LIMIT_MARKER}"),
        headers: json_headers(),
        body: format!("{{\"error\":{{\"message\":\"{OUTPUT_LIMIT_MARKER}\"}}}}"),
    };
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), answer);
    let mut harness = Harness::default();
    harness.model = Some("gpt-4o".to_owned());
    let mut recorder = Recorder::default();
    let reply =
        block_on(async { buffered(harness.run(&source, &mut recorder).await.expect("reply")) });

    let choice = &reply.response.choices[0];
    assert_eq!(choice.finish_reason.as_deref(), Some("length"));
    assert_eq!(choice.message.content.as_deref(), Some(""));
    assert_eq!(reply.response.id, format!("chatcmpl-{}", identifier()));
    assert_eq!(reply.response.created, 1_700_000_000);
    assert_eq!(reply.response.usage.prompt_tokens, 11);
    assert_eq!(reply.response.usage.completion_tokens, 0);
    assert_eq!(reply.response.usage.total_tokens, 11);
    // the recovered reply is logged as the original response like any other
    assert_eq!(recorder.post.len(), 1);
    assert_eq!(source.requests().len(), 1);

    // the async wrapper recovers the same way and keeps the 400's headers on both paths
    let async_source = ScriptedSource::new(
        ClientKind::AsyncAzureOpenAi,
        Some("key"),
        Answer::BadRequest {
            message: format!("Error code: 400 - {OUTPUT_LIMIT_MARKER}"),
            headers: json_headers(),
            body: format!("{{\"error\":{{\"message\":\"{OUTPUT_LIMIT_MARKER}\"}}}}"),
        },
    );
    let mut asynchronous = Harness::default();
    asynchronous.acompletion = true;
    asynchronous.model = Some("gpt-4o".to_owned());
    let mut recorder = Recorder::default();
    let reply = block_on(async {
        buffered(
            asynchronous
                .run(&async_source, &mut recorder)
                .await
                .expect("reply"),
        )
    });
    let choice = &reply.response.choices[0];
    assert_eq!(choice.finish_reason.as_deref(), Some("length"));
    assert_eq!(choice.message.content.as_deref(), Some(""));
    assert_eq!(reply.response.usage.prompt_tokens, 11);
    assert_eq!(recorder.post.len(), 1);
    assert_eq!(
        recorder.response_headers.len(),
        1,
        "the recovered 400 response still stores its headers"
    );
    assert!(reply.hidden_params.get("headers").is_some());
    let Some(Json::Object(additional)) = reply.hidden_params.get("additional_headers") else {
        panic!("additional headers are an object");
    };
    assert_eq!(
        additional.get("llm_provider-content-type"),
        Some(&Json::Text("application/json".to_owned()))
    );
}

pub(super) fn async_timeout_diagnostic() {
    let source = ScriptedSource::new(
        ClientKind::AsyncAzureOpenAi,
        Some("key"),
        Answer::Timeout {
            message: "Request timed out.".to_owned(),
        },
    );
    let mut harness = Harness::default();
    harness.acompletion = true;
    let mut recorder = Recorder::default();
    let error = block_on(async { error_of(harness.run(&source, &mut recorder).await) });
    assert_eq!(status_of(&error), Some(500));
    assert_eq!(
        error.to_string(),
        "Request timed out. - timeout value=30, time taken=0.00 seconds"
    );
    // the async pre call went out before the send, with the call's api_base
    assert_eq!(recorder.pre.len(), 1);
    assert_eq!(
        recorder.pre[0].additional_args.api_base.as_deref(),
        Some("https://unit.test")
    );
    assert_eq!(recorder.post.len(), 1);
    assert_eq!(
        recorder.post[0].original_response,
        Json::Text("Request timed out. - timeout value=30, time taken=0.00 seconds".to_owned())
    );
    assert!(source.gets()[0].is_async);
}

pub(super) fn string_reply_500() {
    let source = ScriptedSource::new(
        ClientKind::AzureOpenAi,
        Some("key"),
        Answer::Reply {
            headers: json_headers(),
            body: "a plain string".to_owned(),
        },
    );
    let harness = Harness::default();
    let mut recorder = Recorder::default();
    let error = block_on(async { error_of(harness.run(&source, &mut recorder).await) });
    assert_eq!(status_of(&error), Some(500));
    assert_eq!(
        error.to_string(),
        "Unexpected string response from Azure: a plain string"
    );
    // the SDK answered, so the call was logged normally
    assert!(recorder.post.is_empty());

    // the async path stores the headers, then logs the failure the string reply raises
    let async_source = ScriptedSource::new(
        ClientKind::AsyncAzureOpenAi,
        Some("key"),
        Answer::Reply {
            headers: json_headers(),
            body: "a plain string".to_owned(),
        },
    );
    let mut asynchronous = Harness::default();
    asynchronous.acompletion = true;
    let mut recorder = Recorder::default();
    let error = block_on(async { error_of(asynchronous.run(&async_source, &mut recorder).await) });
    assert_eq!(status_of(&error), Some(500));
    assert_eq!(
        error.to_string(),
        "Unexpected string response from Azure: a plain string"
    );
    assert_eq!(recorder.post.len(), 1);
    assert_eq!(recorder.response_headers.len(), 1);
}

pub(super) fn client_type_validation() {
    let async_client =
        ScriptedSource::new(ClientKind::AsyncAzureOpenAi, Some("key"), Answer::reply());
    let sync_call = Harness::default();
    let mut recorder = Recorder::default();
    let error = block_on(async { error_of(sync_call.run(&async_client, &mut recorder).await) });
    assert_eq!(status_of(&error), Some(500));
    assert_eq!(error.to_string(), SYNC_CLIENT);

    let sync_client = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut async_call = Harness::default();
    async_call.acompletion = true;
    let mut recorder = Recorder::default();
    let error = block_on(async { error_of(async_call.run(&sync_client, &mut recorder).await) });
    assert_eq!(status_of(&error), Some(500));
    assert_eq!(error.to_string(), ASYNC_CLIENT);
    // the async except block logs str(e) before it wraps the ValueError
    assert_eq!(recorder.post.len(), 1);
    assert!(async_client.requests().is_empty());
    assert!(sync_client.requests().is_empty());
}

pub(super) fn error_status_passthrough() {
    let answer = Answer::Status {
        status: 429,
        message: "rate limited".to_owned(),
        headers: vec![("retry-after".to_owned(), "7".to_owned())],
        body: "{\"error\":{\"message\":\"rate limited\",\"code\":\"429\"}}".to_owned(),
    };
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), answer.clone());
    let harness = Harness::default();
    let mut recorder = Recorder::default();
    let error = block_on(async { error_of(harness.run(&source, &mut recorder).await) });
    assert_eq!(status_of(&error), Some(429));
    let AzureChatError::Azure { headers, body, .. } = &error else {
        panic!("the sync wrapper raises an AzureOpenAIError");
    };
    assert_eq!(
        headers
            .as_ref()
            .and_then(|values| values.0.get("retry-after")),
        Some(&"7".to_owned())
    );
    assert_eq!(
        body.as_deref(),
        Some(&decisions_of(
            json!({"error": {"message": "rate limited", "code": "429"}})
        ))
    );
    // the sync wrapper raises without logging a post call
    assert!(recorder.post.is_empty());
    assert_eq!(source.requests().len(), 1);

    // the async wrapper re-raises the status-bearing exception and the outer completion wraps it
    let async_source = ScriptedSource::new(ClientKind::AsyncAzureOpenAi, Some("key"), answer);
    let mut asynchronous = Harness::default();
    asynchronous.acompletion = true;
    let mut recorder = Recorder::default();
    let error = block_on(async { error_of(asynchronous.run(&async_source, &mut recorder).await) });
    assert_eq!(status_of(&error), Some(429));
    assert_eq!(recorder.post.len(), 1);
    assert_eq!(
        recorder.post[0].original_response,
        Json::Text("rate limited".to_owned())
    );
}

pub(super) fn async_cancellation_passthrough() {
    let source = ScriptedSource::new(ClientKind::AsyncAzureOpenAi, Some("key"), Answer::Pending);
    let mut harness = Harness::default();
    harness.acompletion = true;
    harness.cancel = true;
    let mut recorder = Recorder::default();
    let error = block_on(async { error_of(harness.run(&source, &mut recorder).await) });
    assert!(matches!(error, AzureChatError::Cancelled));
    assert_eq!(recorder.post.len(), 1);
    assert_eq!(
        recorder.post[0].original_response,
        Json::Text(String::new())
    );
    assert_eq!(source.requests().len(), 1);
}

struct SeenForward {
    model: Option<String>,
    api_base: Option<String>,
    api_version: Option<String>,
    api_key: Option<String>,
    acompletion: bool,
    organization: Option<String>,
    drop_params: Option<bool>,
    shared_session: Option<String>,
    client_kind: ClientKind,
    client_api_key: Option<String>,
}

struct Delegate {
    seen: Arc<Mutex<Vec<SeenForward>>>,
}

impl OSeriesDelegate for Delegate {
    type Client = ScriptedTransport;
    type Reply = String;
    type Error = String;

    fn completion<'call>(
        &'call self,
        call: OSeriesForward<'call, Self::Client>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Reply, Self::Error>> + Send + 'call>> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SeenForward {
                model: call.model.map(str::to_owned),
                api_base: call.api_base.map(str::to_owned),
                api_version: call.api_version.map(str::to_owned),
                api_key: call.api_key.map(|key| key.expose_secret().to_owned()),
                acompletion: call.acompletion,
                organization: call.organization.map(str::to_owned),
                drop_params: call.drop_params,
                shared_session: call.shared_session.map(str::to_owned),
                client_kind: call.client.kind,
                client_api_key: call
                    .client
                    .api_key
                    .as_ref()
                    .map(|key| key.expose_secret().to_owned()),
            });
        Box::pin(async { Ok("delegated".to_owned()) })
    }
}

pub(super) fn o_series_client_delegate() {
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let delegate = Delegate {
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let model = "o3-deployment".to_owned();
    let messages = messages_of(json!([{"role": "user", "content": "hi"}]));
    let api_key = SecretString::from("key");
    let optional = Object::new();
    let call = OSeriesCall {
        model: Some(&model),
        messages: Some(&messages),
        api_key: Some(&api_key),
        api_base: Some("https://unit.test"),
        api_version: Some("2024-02-01"),
        azure_ad_token: None,
        timeout: Some(60.0),
        optional_params: &optional,
        litellm_params: LiteLlmParams::default(),
        acompletion: false,
        headers: None,
        organization: Some("org-1"),
        drop_params: Some(true),
        shared_session: Some("session-1"),
        dynamic_params: None,
        supplied: Some(source.handle()),
    };
    let reply = block_on(async { o_series::completion(&source, &call, &delegate).await });
    match reply {
        Ok(text) => assert_eq!(text, "delegated"),
        Err(OSeriesError::Client(_) | OSeriesError::Delegate(_)) => {
            panic!("the delegate returns")
        }
    }
    let gets: Vec<GetRecord> = source.gets();
    assert_eq!(gets.len(), 1);
    // the o-series handler never passes a model to get_azure_openai_client
    assert_eq!(gets[0].model, None);
    assert_eq!(gets[0].api_version.as_deref(), Some("2024-02-01"));
    assert!(gets[0].supplied);
    assert!(source.requests().is_empty());
    let seen = delegate.seen.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(seen[0].model.as_deref(), Some("o3-deployment"));
    assert_eq!(seen[0].api_base.as_deref(), Some("https://unit.test"));
    assert_eq!(seen[0].api_version.as_deref(), Some("2024-02-01"));
    assert_eq!(seen[0].api_key.as_deref(), Some("key"));
    assert!(!seen[0].acompletion);
    assert_eq!(seen[0].organization.as_deref(), Some("org-1"));
    assert_eq!(seen[0].drop_params, Some(true));
    assert_eq!(seen[0].shared_session.as_deref(), Some("session-1"));
    assert_eq!(seen[0].client_kind, ClientKind::AzureOpenAi);
    assert_eq!(seen[0].client_api_key.as_deref(), Some("key"));
}

fn header_answer() -> Answer {
    Answer::Reply {
        headers: vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("llm_provider-x".to_owned(), "kept".to_owned()),
            ("x-custom".to_owned(), "custom".to_owned()),
            ("x-ratelimit-limit-requests".to_owned(), "100".to_owned()),
        ],
        body: DEFAULT_BODY.to_owned(),
    }
}

pub(super) fn raw_headers_projection() {
    let source = ScriptedSource::new(ClientKind::AsyncAzureOpenAi, Some("key"), header_answer());
    let mut harness = Harness::default();
    harness.acompletion = true;
    let mut recorder = Recorder::default();
    let reply =
        block_on(async { buffered(harness.run(&source, &mut recorder).await.expect("reply")) });

    let Json::Object(raw) = reply.hidden_params.get("headers").expect("raw headers") else {
        panic!("raw headers are an object");
    };
    assert_eq!(raw.get("x-custom"), Some(&Json::Text("custom".to_owned())));
    assert_eq!(
        raw.get("llm_provider-x"),
        Some(&Json::Text("kept".to_owned()))
    );
    let additional = reply
        .hidden_params
        .get("additional_headers")
        .expect("additional headers");
    let Json::Object(fields) = additional else {
        panic!("additional headers are an object");
    };
    assert_eq!(
        fields.get("llm_provider-x"),
        Some(&Json::Text("kept".to_owned()))
    );
    assert_eq!(
        fields.get("llm_provider-x-custom"),
        Some(&Json::Text("custom".to_owned()))
    );
    assert_eq!(
        fields.get("x-ratelimit-limit-requests"),
        Some(&Json::Text("100".to_owned()))
    );
    assert_eq!(
        recorder.response_headers.len(),
        1,
        "the async path stores the response headers for logging"
    );
    assert_eq!(
        recorder.response_headers[0]
            .get("x-custom")
            .map(String::as_str),
        Some("custom")
    );

    // the sync path keeps only the derived additional headers
    let sync_source = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), header_answer());
    let sync_call = Harness::default();
    let mut recorder = Recorder::default();
    let reply = block_on(async {
        buffered(
            sync_call
                .run(&sync_source, &mut recorder)
                .await
                .expect("reply"),
        )
    });
    assert!(reply.hidden_params.get("headers").is_none());
    assert!(recorder.response_headers.is_empty());
    let Json::Object(fields) = reply
        .hidden_params
        .get("additional_headers")
        .expect("additional headers")
    else {
        panic!("additional headers are an object");
    };
    assert_eq!(
        fields.get("llm_provider-x"),
        Some(&Json::Text("kept".to_owned()))
    );
    assert_eq!(
        fields.get("x-ratelimit-limit-requests"),
        Some(&Json::Text("100".to_owned()))
    );
}

const STREAM_EVENTS: &str = concat!(
    "data: {\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o-deployment\",",
    "\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hello\"},",
    "\"finish_reason\":null}]}\n\n",
    "data: [DONE]\n\n"
);

#[test]
pub(super) fn streaming_deployment_request() {
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply())
        .with_stream(STREAM_EVENTS);
    let mut harness = Harness::default();
    harness.optional_params = json!({"stream": true});
    let mut recorder = Recorder::default();
    let outcome = block_on(async {
        harness
            .run(&source, &mut recorder)
            .await
            .expect("stream opens")
    });
    let mut stream = match outcome {
        ChatOutcome::Stream(stream) => stream,
        ChatOutcome::Buffered(_) => panic!("the streaming case opens a stream"),
    };
    assert_eq!(stream.model, MODEL);
    assert!(
        stream
            .stream_options
            .as_ref()
            .is_some_and(|options| options.include_usage)
    );
    assert_eq!(recorder.pre.len(), 1);
    assert_eq!(recorder.response_headers.len(), 1);
    let chunks = block_on(async move {
        let mut values = Vec::new();
        while let Some(item) = stream.chunks.next().await {
            values.push(item.expect("chunk"));
        }
        values
    });
    assert_eq!(chunks[0].id, format!("chatcmpl-{}", identifier()));
    assert_eq!(chunks[0].created, 1_700_000_000);
    assert_eq!(chunks[0].choices[0].delta.content.as_deref(), Some("Hello"));

    // the sync path streams only on a literal true, so another truthy value still buffers
    let buffered_source =
        ScriptedSource::new(ClientKind::AzureOpenAi, Some("key"), Answer::reply());
    let mut truthy = Harness::default();
    truthy.optional_params = json!({"stream": 1});
    let mut recorder = Recorder::default();
    let outcome = block_on(async {
        truthy
            .run(&buffered_source, &mut recorder)
            .await
            .expect("reply")
    });
    assert!(matches!(outcome, ChatOutcome::Buffered(_)));

    // acompletion streams on any truthy stream value, so a number is enough there
    let async_source =
        ScriptedSource::new(ClientKind::AsyncAzureOpenAi, Some("key"), Answer::reply())
            .with_stream(STREAM_EVENTS);
    let mut asynchronous = Harness::default();
    asynchronous.acompletion = true;
    asynchronous.optional_params = json!({"stream": 1});
    let mut recorder = Recorder::default();
    let outcome = block_on(async {
        asynchronous
            .run(&async_source, &mut recorder)
            .await
            .expect("stream opens")
    });
    assert!(matches!(outcome, ChatOutcome::Stream(_)));
}
