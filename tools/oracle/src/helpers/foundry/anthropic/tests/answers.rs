use std::convert::Infallible;
use std::sync::Mutex;

use betterlite_anthropic::anthropic_http_stream::completion::WrappedStream;
use betterlite_anthropic::anthropic_http_stream::transport::StreamPost;
use betterlite_anthropic::chat_call::ChatOutcome;
use betterlite_anthropic::chat_call::{ChatClients, ChatDeps};
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_anthropic::stream_decode::DecodedChunk;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_foundry::anthropic::error::ChatError;
use betterlite_foundry::anthropic::handler::completion as provider_completion;
use betterlite_foundry::anthropic::request::{ChatRequest, Style};
use betterlite_foundry::anthropic_messages::environment::{ParamsKey, TokenSource};
use betterlite_foundry::chat::sources::NoSources;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::client_kind::SuppliedClient;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use futures_util::StreamExt as _;
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tokio::runtime::Runtime as AsyncRuntime;

use super::fixture::{MAX_BYTES, Mapper, NOW, Quiet, Runtime, TIMEOUT, Tally, Wire, WireFailure};
use super::input::Input;

const HEADERS: [&str; 5] = [
    "api-key",
    "x-api-key",
    "authorization",
    "anthropic-version",
    "anthropic-beta",
];
const BODY_KEYS: [&str; 11] = [
    "max_tokens",
    "temperature",
    "top_p",
    "thinking",
    "extra_body",
    "max_retries",
    "stream_options",
    "stream",
    "json_mode",
    "is_vertex_request",
    "system",
];

pub(crate) fn completion(input: &Input) -> HelperResult {
    let wire = Wire {
        status: input.upstream_status.unwrap_or(200),
        headers: header_map(&input.upstream_headers)?,
        body: input.upstream_body.clone().unwrap_or_default().into_bytes(),
        lines: input.upstream_lines.clone(),
        posts: Mutex::new(Vec::new()),
    };
    let argument = input.api_key.clone().map(SecretString::from);
    let credentials = credentials(input);
    let request = request_of(input, argument.as_ref(), &credentials);
    let fixture = Runtime::new()?;
    let mut state = fixture.state()?;
    let env = fixture.env();
    let quiet = Quiet;
    let config = Mapper;
    let deps = ChatDeps {
        config: &config,
        clients: ChatClients {
            supplied: Some(SuppliedClient {
                kind: ClientKind::Sync,
                transport: &wire,
            }),
            pooled: &wire,
        },
        state: &mut state,
        env: &env,
        stream_logger: &quiet,
        counter: Tally,
        ids: &quiet,
        max_event_bytes: MAX_BYTES,
        max_error_body_bytes: MAX_BYTES,
    };
    let executor = Builder::new_current_thread().build().or_value_error()?;
    let outcome = executor.block_on(provider_completion(request, deps));
    outcome_value(&executor, outcome, &wire)
}

// The consumed stream and the wrapper's identity, which a case compares against LiteLLM's own chunks
fn stream_value(
    executor: &AsyncRuntime,
    stream: WrappedStream<WireFailure, Tally>,
) -> HelperResult {
    let model = stream.model.clone();
    let provider = stream.custom_llm_provider;
    let (text, finish_reasons) = executor.block_on(async move {
        let mut chunks = stream.completion_stream;
        let mut text = String::new();
        let mut finish_reasons: Vec<String> = Vec::new();
        while let Some(item) = chunks.next().await {
            let DecodedChunk::Event(event) = item.or_value_error()? else {
                continue;
            };
            for choice in &event.choices {
                text.push_str(&choice.delta.content);
                if let Some(reason) = &choice.finish_reason {
                    finish_reasons.push(reason.clone());
                }
            }
        }
        Ok::<_, Box<HelperError>>((text, finish_reasons))
    })?;
    Ok(json!({
        "model": model,
        "provider": provider,
        "text": text,
        "finish_reasons": finish_reasons,
    }))
}

fn request_of<'call>(
    input: &'call Input,
    argument: Option<&'call SecretString>,
    credentials: &'call DeploymentSettings,
) -> ChatRequest<'call> {
    ChatRequest {
        model: input.model.as_str(),
        messages: input.messages.clone(),
        optional_params: input.optional_params.clone(),
        litellm_params: input.litellm_params.clone(),
        caller_headers: input.caller_headers.clone(),
        api_base: input.api_base.as_str(),
        params_key: ParamsKey::DictAbsent,
        argument_key: argument,
        configured_key: None,
        token: TokenSource {
            credentials,
            refresh_enabled: false,
            provider: None,
            sources: &NoSources,
        },
        timeout: TIMEOUT,
        style: Style::Sync,
        now: NOW,
        disable_add_prefix_to_prompt: false,
        model_response: ResponseEnvelope::new("chatcmpl-1".to_owned()),
    }
}

fn outcome_value(
    executor: &AsyncRuntime,
    outcome: Result<ChatOutcome<WireFailure, Tally>, ChatError<WireFailure, Infallible>>,
    wire: &Wire,
) -> HelperResult {
    let recorded = recorded_parts(wire);
    let stream = match outcome {
        Ok(ChatOutcome::Reply(_)) => None,
        Ok(ChatOutcome::Stream(wrapped)) => Some(stream_value(executor, *wrapped)?),
        Ok(ChatOutcome::Embedding) => {
            return Err("a chat call has no embedding outcome".to_owned()).or_value_error();
        }
        // the handler raises its AnthropicError with only a status, so the other fields are empty
        Err(error) => {
            let status = error.status_code();
            return Err(Box::new(HelperError {
                class: "AnthropicError".to_owned(),
                status,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message: error.to_string(),
            }));
        }
    };
    Ok(json!({
        "url": recorded.url,
        "headers": recorded.headers,
        "body": recorded.body,
        "stream": stream,
    }))
}

// What the endpoint received, projected the way a case's answer compares it
struct Recorded {
    url: Value,
    headers: IndexMap<&'static str, Option<String>>,
    body: IndexMap<&'static str, Value>,
}

fn recorded_parts(wire: &Wire) -> Recorded {
    let posts = wire.recorded();
    let url = json!(posts.first().map(|post| post.url.as_str()));
    let mut headers = IndexMap::new();
    for name in HEADERS {
        headers.insert(
            name,
            posts.first().and_then(|post| header_value(post, name)),
        );
    }
    let body = posts.first().map_or(Value::Null, |post| {
        serde_json::from_slice(&post.body).unwrap_or(Value::Null)
    });
    drop(posts);
    let mut fields = IndexMap::new();
    for name in BODY_KEYS {
        if let Some(value) = body.get(name) {
            fields.insert(name, value.clone());
        }
    }
    Recorded {
        url,
        headers,
        body: fields,
    }
}

// LiteLLM joins derived betas out of a set, so the order says nothing about the provider
fn header_value(post: &StreamPost, name: &str) -> Option<String> {
    let value = post.headers.get(name)?.to_str().ok()?;
    let mut parts: Vec<&str> = value.split(',').collect();
    parts.sort_unstable();
    Some(parts.join(","))
}

fn header_map(pairs: &IndexMap<String, String>) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.insert(
            HeaderName::try_from(name.as_str()).or_value_error()?,
            HeaderValue::try_from(value.as_str()).or_value_error()?,
        );
    }
    Ok(headers)
}

fn credentials(input: &Input) -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: input.azure_ad_token.clone().map(SecretString::from),
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: "https://cognitiveservices.azure.com/.default".to_owned(),
        azure_username: None,
        azure_password: None,
    }
}
