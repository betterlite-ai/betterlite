use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_prompts::arguments::json::Json;
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

    fn answer_a2a(
        &self,
        call: &CompletionCall,
        status: u16,
        body: &[u8],
    ) -> Result<Value, HandlerError> {
        let headers = BTreeMap::new();
        let reply = self.runtime.block_on(a2a_reply::transform_response(
            &a2a_reply::ReplyInput {
                model: &call.model,
                status,
                headers: &headers,
                body,
                messages: &call.messages,
            },
            &RandomIds,
            &Meter,
        ))?;
        let Some(choice) = reply.choices.first() else {
            return Err(HandlerError::EmptyReply);
        };
        Ok(json!({
            "model": reply.model,
            "content": choice.message.content,
            "role": choice.message.role,
            "finish_reason": choice.finish_reason,
        }))
    }

    fn answer_langgraph(
        &self,
        call: &CompletionCall,
        status: u16,
        body: &[u8],
    ) -> Result<Value, HandlerError> {
        let reply = self.runtime.block_on(langgraph_reply::transform_response(
            &langgraph_reply::ReplyInput {
                model: &call.model,
                status,
                body,
                messages: &call.messages,
            },
            &Meter,
        ))?;
        let Some(choice) = reply.choices.first() else {
            return Err(HandlerError::EmptyReply);
        };
        Ok(json!({
            "model": reply.model,
            "content": choice.content,
            "role": role_text(choice.role),
            "finish_reason": choice.finish_reason,
        }))
    }

    fn answer_langflow(
        &self,
        call: &CompletionCall,
        status: u16,
        body: &[u8],
    ) -> Result<Value, HandlerError> {
        let reply = self.runtime.block_on(langflow_reply::transform_response(
            &langflow_reply::ReplyInput {
                model: &call.model,
                status,
                body,
                messages: &call.messages,
            },
            &Meter,
        ))?;
        Ok(json!({
            "model": reply.model,
            "content": reply.choice.message.content,
            "role": reply.choice.message.role,
            "finish_reason": reply.choice.finish_reason,
        }))
    }
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
        let body = self.upstream.body.as_bytes();
        match self.platform {
            Platform::A2a => self.answer_a2a(call, status, body),
            Platform::Langgraph => self.answer_langgraph(call, status, body),
            Platform::Langflow => self.answer_langflow(call, status, body),
        }
    }
}

fn a2a_request_value(call: &CompletionCall) -> Result<Value, HandlerError> {
    let url = a2a_request::get_complete_url(Some(call.api_base.as_str()))?;
    let headers = call.headers.clone().unwrap_or_default();
    let validated = a2a_request::validate_environment(&headers, call.api_key.as_ref());
    let body = a2a_request::transform_request(&call.messages, &call.optional_params, &RandomIds)?;
    Ok(json!({"url": url, "headers": header_view(&validated), "body": body}))
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
    Ok(json!({"url": url, "headers": header_view(&headers), "body": body}))
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
    let request = json!({
        "url": prepared.url,
        "headers": header_view(&prepared.headers),
        "body": prepared.body,
    });
    Ok(request)
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
