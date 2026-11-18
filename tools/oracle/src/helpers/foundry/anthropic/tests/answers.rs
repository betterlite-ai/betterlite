use std::convert::Infallible;
use std::sync::Mutex;

use betterlite_anthropic::anthropic_http_stream::transport::StreamPost;
use betterlite_anthropic::chat_call::ChatOutcome;
use betterlite_anthropic::chat_call::transport::{ClientKind, SuppliedChatClient};
use betterlite_anthropic::chat_call::{ChatClients, ChatDeps};
use betterlite_anthropic::response::wire::ResponseEnvelope;
use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_foundry::anthropic::error::ChatError;
use betterlite_foundry::anthropic::handler::completion as provider_completion;
use betterlite_foundry::anthropic::request::{ChatRequest, Style};
use betterlite_foundry::anthropic_messages::environment::{ParamsKey, TokenSource};
use betterlite_foundry::chat::sources::NoSources;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::fixture::{MAX_BYTES, Mapper, NOW, Quiet, Runtime, TIMEOUT, Tally, Wire, WireFailure};
use super::input::Input;

const HEADERS: [&str; 5] = [
    "api-key",
    "x-api-key",
    "authorization",
    "anthropic-version",
    "anthropic-beta",
];
const BODY_KEYS: [&str; 10] = [
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
    let request = request_of(input, &argument, &credentials);
    let fixture = Runtime::new()?;
    let mut state = fixture.state()?;
    let env = fixture.env();
    let quiet = Quiet;
    let config = Mapper;
    let deps = ChatDeps {
        config: &config,
        clients: ChatClients {
            supplied: Some(SuppliedChatClient {
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
    outcome_value(executor.block_on(provider_completion(request, deps)), &wire)
}

fn request_of<'call>(
    input: &'call Input,
    argument: &'call Option<SecretString>,
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
        argument_key: argument.as_ref(),
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
    outcome: Result<ChatOutcome<WireFailure, Tally>, ChatError<WireFailure, Infallible>>,
    wire: &Wire,
) -> HelperResult {
    let recorded = wire.recorded();
    let url = json!(recorded.first().map(|post| post.url.as_str()));
    let mut headers = IndexMap::new();
    for name in HEADERS {
        headers.insert(
            name,
            recorded.first().and_then(|post| header_value(post, name)),
        );
    }
    let body = recorded.first().map_or(Value::Null, |post| {
        serde_json::from_slice(&post.body).unwrap_or(Value::Null)
    });
    let mut fields = IndexMap::new();
    for name in BODY_KEYS {
        if let Some(value) = body.get(name) {
            fields.insert(name, value.clone());
        }
    }
    match outcome {
        Ok(ChatOutcome::Reply(_) | ChatOutcome::Stream(_)) => Ok(json!({
            "url": url,
            "headers": headers,
            "body": fields,
        })),
        Ok(ChatOutcome::Embedding) => {
            Err("a chat call has no embedding outcome".to_owned()).or_value_error()
        }
        Err(error) => {
            let status = error.status_code();
            Err(Box::new(HelperError {
                class: "AnthropicError".to_owned(),
                status,
                error_type: Some(None),
                param: Some(None),
                code: Some(None),
                message: error.to_string(),
            }))
        }
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
