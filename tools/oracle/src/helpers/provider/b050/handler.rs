use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_prompts::arguments::json::Json;
use betterlite_provider::a2a_chat::records::A2aReply;
use betterlite_provider::a2a_chat::reply as a2a_reply;
use betterlite_provider::a2a_chat::request as a2a_request;
use betterlite_provider::agent_platform_dispatch::call::CompletionCall;
use betterlite_provider::agent_platform_dispatch::handler::{ChatCompletionHandler, HandlerError};
use betterlite_provider::langflow_chat::call as langflow_call;
use betterlite_provider::langflow_chat::reply as langflow_reply;
use betterlite_provider::langgraph_chat::config as langgraph_config;
use betterlite_provider::langgraph_chat::records::LangGraphRole;
use betterlite_provider::langgraph_chat::reply as langgraph_reply;
use betterlite_provider::langgraph_chat::request as langgraph_request;
use betterlite_provider::ollama_chat::ids::RandomIds;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

use super::script::{Platform, UpstreamCase};
use super::stubs::Meter;

const KEPT_HEADERS: [&str; 4] = ["authorization", "content-type", "x-api-key", "x-card"];

pub(super) struct Handler {
    platform: Platform,
    upstream: UpstreamCase,
    runtime: Runtime,
    requests: Mutex<Vec<Value>>,
}

impl Handler {
    pub(super) fn new(
        platform: Platform,
        upstream: &UpstreamCase,
    ) -> Result<Self, Box<HelperError>> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_config_error()?;
        Ok(Self {
            platform,
            upstream: upstream.clone(),
            runtime,
            requests: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer(&self, call: &CompletionCall, status: u16) -> Result<Answer, HandlerError> {
        let body = self.upstream.body.as_bytes();
        match self.platform {
            Platform::A2a => Ok(Answer::A2a(self.runtime.block_on(
                a2a_reply::transform_response(
                    &a2a_reply::ReplyInput {
                        model: &call.model,
                        status,
                        headers: &BTreeMap::new(),
                        body,
                        messages: &call.messages,
                    },
                    &RandomIds,
                    &Meter,
                ),
            )?)),
            Platform::Langgraph => Ok(Answer::Langgraph(self.runtime.block_on(
                langgraph_reply::transform_response(
                    &langgraph_reply::ReplyInput {
                        model: &call.model,
                        status,
                        body,
                        messages: &call.messages,
                    },
                    &Meter,
                ),
            )?)),
            Platform::Langflow => Ok(Answer::Langflow(self.runtime.block_on(
                langflow_reply::transform_response(
                    &langflow_reply::ReplyInput {
                        model: &call.model,
                        status,
                        body,
                        messages: &call.messages,
                    },
                    &Meter,
                ),
            )?)),
        }
    }
}

// the three replies hold their fields in different places, so the compared projection matches on the platform
enum Answer {
    A2a(A2aReply),
    Langgraph(langgraph_reply::LangGraphReply),
    Langflow(langflow_reply::LangFlowChatResponse),
}

impl Answer {
    fn into_value(self) -> Result<Value, HandlerError> {
        match self {
            Self::A2a(reply) => {
                let Some(choice) = reply.choices.into_iter().next() else {
                    return Err(HandlerError::EmptyReply);
                };
                let content = Some(choice.message.content.as_str());
                Ok(reply_value(
                    &reply.model,
                    content,
                    choice.message.role,
                    choice.finish_reason,
                ))
            }
            Self::Langgraph(reply) => {
                let Some(choice) = reply.choices.into_iter().next() else {
                    return Err(HandlerError::EmptyReply);
                };
                Ok(reply_value(
                    &reply.model,
                    choice.content.as_deref(),
                    role_text(choice.role),
                    choice.finish_reason,
                ))
            }
            Self::Langflow(reply) => {
                let message = &reply.choice.message;
                let content = Some(message.content.as_str());
                Ok(reply_value(
                    &reply.model,
                    content,
                    &message.role,
                    &reply.choice.finish_reason,
                ))
            }
        }
    }
}

// the compared projection of a reply: its model, the first choice's content and role, and the finish reason
fn reply_value(model: &str, content: Option<&str>, role: &str, finish_reason: &str) -> Value {
    json!({
        "model": model,
        "content": content,
        "role": role,
        "finish_reason": finish_reason,
    })
}

impl ChatCompletionHandler for Handler {
    type Reply = Value;

    fn complete(&self, call: &CompletionCall) -> Result<Value, HandlerError> {
        let request = match self.platform {
            Platform::A2a => a2a_request_value(call)?,
            Platform::Langgraph => langgraph_request_value(call)?,
            Platform::Langflow => langflow_request_value(call)?,
        };
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let status = self.upstream.status;
        if status >= 400 {
            // LiteLLM's shared handler turns a 4xx or 5xx into the provider error before any reply transform
            return Err(HandlerError::Upstream {
                status_code: status,
            });
        }
        if call.stream == Some(true) {
            // a streaming call hands back a stream rather than a reply, and this harness carries no chunks
            return Ok(Value::Null);
        }
        self.answer(call, status)?.into_value()
    }
}

fn a2a_request_value(call: &CompletionCall) -> Result<Value, HandlerError> {
    let url = a2a_request::get_complete_url(Some(call.api_base.as_str()))?;
    let headers = call.headers.clone().unwrap_or_default();
    let validated = a2a_request::validate_environment(&headers, call.api_key.as_ref());
    let body = a2a_request::transform_request(&call.messages, &call.optional_params, &RandomIds)?;
    Ok(request_view(&url, &validated, &body))
}

fn langgraph_request_value(call: &CompletionCall) -> Result<Value, HandlerError> {
    let stream = call.stream == Some(true);
    let url = langgraph_config::get_complete_url(Some(call.api_base.as_str()), stream)?;
    let mut headers = call.headers.clone().unwrap_or_default();
    langgraph_config::validate_environment(&mut headers, call.api_key.as_ref());
    // LiteLLM reads the body's stream_mode flag from litellm_params, while the endpoint follows the call's stream flag
    let body_stream = matches!(call.litellm_params.get("stream"), Some(Json::Bool(true)));
    let body = langgraph_request::transform_request(
        &call.model,
        &call.messages,
        &call.optional_params,
        body_stream,
    )?;
    Ok(request_view(&url, &headers, &body))
}

fn langflow_request_value(call: &CompletionCall) -> Result<Value, HandlerError> {
    let prepared = langflow_call::prepare_run(&langflow_call::RunInput {
        model: &call.model,
        messages: &call.messages,
        optional_params: &call.optional_params,
        api_base: Some(call.api_base.as_str()),
        api_key: call.api_key.as_ref(),
        stream: call.stream,
    })?;
    Ok(request_view(
        &prepared.url,
        &prepared.headers,
        &prepared.body,
    ))
}

// the compared projection of an outgoing request: its url, the headers worth comparing and the body
fn request_view<B>(url: &str, headers: &BTreeMap<String, String>, body: &B) -> Value
where
    B: Serialize,
{
    json!({
        "url": url,
        "headers": header_view(headers),
        "body": body,
    })
}

fn header_view(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter(|(name, _)| KEPT_HEADERS.contains(&name.to_ascii_lowercase().as_str()))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect()
}

const fn role_text(role: LangGraphRole) -> &'static str {
    match role {
        LangGraphRole::Assistant => "assistant",
        LangGraphRole::Human => "human",
        LangGraphRole::System => "system",
    }
}
