use std::time::Duration;

use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::provider::a2a::JsonRpcId;
use betterlite_logging::logging_state::correlation::CorrelationSetup;
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, Messages, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState, LoggingStateError};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use serde_json::Number;
use thiserror::Error;

use crate::agent_card::{AgentCard, WireValue};

mod tests;

pub const A2A_COST_PARAM_KEYS: [&str; 3] = [
    "cost_per_query",
    "input_cost_per_token",
    "output_cost_per_token",
];

const A2A_CUSTOM_LLM_PROVIDER: &str = "a2a_agent";
const STREAMING_CALL_TYPE: &str = "asend_message_streaming";

#[derive(Debug, Error)]
pub enum LoggingContextError {
    #[error("the A2A usage token total overflows")]
    UsageOverflow,
    #[error("the A2A logging {field} value must be an object")]
    NotObject { field: &'static str },
    #[error(transparent)]
    State(#[from] LoggingStateError),
}

pub(crate) fn usage_json(
    prompt_tokens: u64,
    completion_tokens: u64,
) -> Result<Json, LoggingContextError> {
    let total_tokens = prompt_tokens
        .checked_add(completion_tokens)
        .ok_or(LoggingContextError::UsageOverflow)?;
    let usage = [
        ("prompt_tokens", prompt_tokens),
        ("completion_tokens", completion_tokens),
        ("total_tokens", total_tokens),
    ]
    .into_iter()
    .map(|(name, count)| (name.to_owned(), Json::Number(Number::from(count))))
    .collect();
    Ok(Json::Object(usage))
}

pub fn set_usage_on_logging_obj(
    logging: Option<&mut LoggingState>,
    prompt_tokens: u64,
    completion_tokens: u64,
) -> Result<(), LoggingContextError> {
    let Some(state) = logging else {
        return Ok(());
    };
    let usage = usage_json(prompt_tokens, completion_tokens)?;
    state.model_call_details.insert("usage".to_owned(), usage);
    Ok(())
}

pub fn set_agent_id_on_logging_obj(logging: Option<&mut LoggingState>, agent_id: Option<&str>) {
    let (Some(state), Some(id)) = (logging, agent_id) else {
        return;
    };
    state
        .model_call_details
        .insert("agent_id".to_owned(), Json::Text(id.to_owned()));
}

#[must_use]
pub fn a2a_cost_params(litellm_params: Option<&IndexMap<String, Json>>) -> IndexMap<String, Json> {
    let mut cost_params = IndexMap::new();
    let Some(params) = litellm_params else {
        return cost_params;
    };
    for key in A2A_COST_PARAM_KEYS {
        if let Some(value) = params.get(key).filter(|value| !matches!(value, Json::Null)) {
            cost_params.insert(key.to_owned(), value.clone());
        }
    }
    cost_params
}

pub fn set_litellm_params(
    logging: Option<&mut LoggingState>,
    litellm_params: &IndexMap<String, Json>,
) -> Result<(), LoggingContextError> {
    let Some(state) = logging else {
        return Ok(());
    };
    let cost_params = a2a_cost_params(Some(litellm_params));
    if cost_params.is_empty() {
        return Ok(());
    }
    let mut merged = match state.model_call_details.get("litellm_params") {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Object(existing)) => existing.clone(),
        Some(_) => {
            return Err(LoggingContextError::NotObject {
                field: "litellm_params",
            });
        }
    };
    merged.extend(cost_params);
    state
        .model_call_details
        .insert("litellm_params".to_owned(), Json::Object(merged));
    Ok(())
}

pub fn get_a2a_model_info(
    agent_card: Option<&AgentCard>,
    logging: Option<&mut LoggingState>,
) -> String {
    let agent_name = agent_card
        .and_then(card_name)
        .unwrap_or("unknown")
        .to_owned();
    let model = format!("a2a_agent/{agent_name}");
    if let Some(state) = logging {
        state.model.clone_from(&model);
        state.custom_llm_provider = Some(A2A_CUSTOM_LLM_PROVIDER.to_owned());
        state
            .model_call_details
            .insert("model".to_owned(), Json::Text(model));
        state.model_call_details.insert(
            "custom_llm_provider".to_owned(),
            Json::Text(A2A_CUSTOM_LLM_PROVIDER.to_owned()),
        );
    }
    agent_name
}

fn card_name(agent_card: &AgentCard) -> Option<&str> {
    match agent_card.name.as_ref()?.as_ref()? {
        WireValue::Typed(name) | WireValue::Passthrough(AgentJson::String(name)) => Some(name),
        WireValue::Passthrough(_) => None,
    }
}

pub(crate) fn json_rpc_id_text(id: &JsonRpcId) -> String {
    match id {
        JsonRpcId::String(text) => text.clone(),
        JsonRpcId::Integer(number_id) => Number::from(number_id).to_string(),
    }
}

pub(crate) fn json_rpc_id_json(id: &JsonRpcId) -> Json {
    match id {
        JsonRpcId::String(text) => Json::Text(text.clone()),
        JsonRpcId::Integer(number_id) => Json::Number(Number::from(number_id)),
    }
}

pub struct StreamingLoggingRequest<'request> {
    pub request_id: &'request JsonRpcId,
    pub agent_name: &'request str,
    pub agent_id: Option<&'request str>,
    pub litellm_params: Option<&'request IndexMap<String, Json>>,
    pub metadata: Option<&'request Json>,
    pub proxy_server_request: Option<&'request Json>,
}

pub struct LoggingDeps<'deps, Clock> {
    pub start_time: DateTime<FixedOffset>,
    pub correlation: CorrelationSetup<'deps>,
    pub redactor: &'deps Redactor,
    pub generated_trace_id: &'deps str,
    pub clock: Clock,
}

pub fn build_streaming_logging_obj<Clock>(
    request: &StreamingLoggingRequest<'_>,
    deps: LoggingDeps<'_, Clock>,
) -> Result<LoggingState, LoggingContextError>
where
    Clock: FnMut() -> Duration,
{
    let generated_trace_id = deps.generated_trace_id;
    let request_id = json_rpc_id_text(request.request_id);
    let model = format!("a2a_agent/{}", request.agent_name);
    let init = streaming_logging_init(&request_id, &model, deps.start_time);
    let mut logging = LoggingState::new(
        init,
        deps.correlation,
        deps.redactor,
        || generated_trace_id.to_owned(),
        deps.clock,
    )?;
    let metadata = object_param(request.metadata, "metadata")?;
    let params = streaming_params(request, metadata)?;
    apply_streaming_logging(&mut logging, request, &model, params, metadata);
    Ok(logging)
}

fn streaming_logging_init(
    request_id: &str,
    model: &str,
    start_time: DateTime<FixedOffset>,
) -> LoggingInit {
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("user".to_owned()));
    message.insert(
        "content".to_owned(),
        Json::Text("streaming-request".to_owned()),
    );
    LoggingInit {
        model: model.to_owned(),
        call_type: STREAMING_CALL_TYPE.to_owned(),
        start_time,
        litellm_call_id: request_id.to_owned(),
        function_id: request_id.to_owned(),
        messages: Some(Messages::Values(vec![Json::Object(message)])),
        stream: Some(false),
        litellm_trace_id: None,
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    }
}

fn streaming_params(
    request: &StreamingLoggingRequest<'_>,
    metadata: Option<&IndexMap<String, Json>>,
) -> Result<IndexMap<String, Json>, LoggingContextError> {
    let mut params = a2a_cost_params(request.litellm_params);
    if let Some(fields) = metadata {
        params.insert("metadata".to_owned(), Json::Object(fields.clone()));
    }
    if let Some(proxy) = object_param(request.proxy_server_request, "proxy_server_request")? {
        params.insert(
            "proxy_server_request".to_owned(),
            Json::Object(proxy.clone()),
        );
    }
    Ok(params)
}

fn apply_streaming_logging(
    logging: &mut LoggingState,
    request: &StreamingLoggingRequest<'_>,
    model: &str,
    params: IndexMap<String, Json>,
    metadata: Option<&IndexMap<String, Json>>,
) {
    let call_type = logging.call_type.clone();
    model.clone_into(&mut logging.model);
    logging.custom_llm_provider = Some(A2A_CUSTOM_LLM_PROVIDER.to_owned());
    logging.litellm_params.clone_from(&params);
    logging.optional_params.clone_from(&params);
    let metadata_fields = metadata.cloned().unwrap_or_default();
    let details = &mut logging.model_call_details;
    details.insert("model".to_owned(), Json::Text(model.to_owned()));
    details.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(A2A_CUSTOM_LLM_PROVIDER.to_owned()),
    );
    details.insert("call_type".to_owned(), Json::Text(call_type));
    if let Some(agent_id) = request.agent_id {
        details.insert("agent_id".to_owned(), Json::Text(agent_id.to_owned()));
    }
    details.insert("litellm_params".to_owned(), Json::Object(params));
    details.insert("metadata".to_owned(), Json::Object(metadata_fields));
}

const fn object_param<'value>(
    value: Option<&'value Json>,
    field: &'static str,
) -> Result<Option<&'value IndexMap<String, Json>>, LoggingContextError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(_) => Err(LoggingContextError::NotObject { field }),
    }
}
