use std::fmt::Display;
use std::time::Duration;

use betterlite_a2a::agent_card::AgentCard;
use betterlite_a2a::client::records::compat::SendMessageRequest;
use betterlite_a2a::logging_context::{
    LoggingDeps, StreamingLoggingRequest, a2a_cost_params, build_streaming_logging_obj,
    get_a2a_model_info, set_agent_id_on_logging_obj, set_usage_on_logging_obj,
};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;
use serde::Deserialize;

use crate::compare::helper::HelperError;
use crate::helpers::{Helper, HelperResult};

mod state;
mod streaming;
mod tokenizer;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    SetUsage {
        instances: Vec<UsageInstance>,
    },
    SetAgentId {
        instances: Vec<AgentIdInstance>,
    },
    CostParams {
        instances: Vec<CostParamsInstance>,
    },
    ModelInfo {
        instances: Vec<ModelInfoInstance>,
    },
    BuildStreaming {
        instances: Vec<BuildStreamingInstance>,
    },
    Iterate {
        instances: Vec<streaming::IterateInstance>,
    },
}

const fn default_logging() -> bool {
    true
}

#[derive(Deserialize)]
pub(crate) struct UsageInstance {
    #[serde(default = "default_logging")]
    logging: bool,
    #[serde(default)]
    details: IndexMap<String, Json>,
    prompt_tokens: u64,
    completion_tokens: u64,
}

#[derive(Deserialize)]
pub(crate) struct AgentIdInstance {
    #[serde(default = "default_logging")]
    logging: bool,
    #[serde(default)]
    details: IndexMap<String, Json>,
    #[serde(default)]
    agent_id: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CostParamsInstance {
    #[serde(default)]
    litellm_params: Option<IndexMap<String, Json>>,
}

#[derive(Deserialize)]
pub(crate) struct ModelInfoInstance {
    #[serde(default = "default_logging")]
    logging: bool,
    #[serde(default)]
    details: IndexMap<String, Json>,
    #[serde(default)]
    card: Option<AgentCard>,
}

#[derive(Deserialize)]
pub(crate) struct BuildStreamingInstance {
    request: SendMessageRequest,
    agent_name: String,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    litellm_params: Option<IndexMap<String, Json>>,
    #[serde(default)]
    metadata: Option<Json>,
    #[serde(default)]
    proxy_server_request: Option<Json>,
}

pub(crate) fn helper_error<M>(class: &str, message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

fn run_all<Instance>(
    instances: &[Instance],
    operation: fn(&Instance) -> HelperResult,
) -> HelperResult {
    let mut results = Vec::new();
    for instance in instances {
        results.push(operation(instance)?);
    }
    Ok(serde_json::json!({ "results": results }))
}

fn details_or_null(logging: &LoggingState, present: bool) -> HelperResult {
    if present {
        return Ok(serde_json::json!({ "details": state::details_value(logging)? }));
    }
    Ok(serde_json::json!({ "details": serde_json::Value::Null }))
}

fn mutate_logging<Operation>(
    details: &IndexMap<String, Json>,
    present: bool,
    operation: Operation,
) -> HelperResult
where
    Operation: FnOnce(Option<&mut LoggingState>) -> Result<(), Box<HelperError>>,
{
    let mut logging = state::state(details)?;
    operation(present.then_some(&mut logging))?;
    details_or_null(&logging, present)
}

fn usage_result(instance: &UsageInstance) -> HelperResult {
    mutate_logging(&instance.details, instance.logging, |logging| {
        set_usage_on_logging_obj(logging, instance.prompt_tokens, instance.completion_tokens)
            .map_err(|error| helper_error("LoggingContextError", error.to_string()))
    })
}

fn agent_id_result(instance: &AgentIdInstance) -> HelperResult {
    mutate_logging(&instance.details, instance.logging, |logging| {
        set_agent_id_on_logging_obj(logging, instance.agent_id.as_deref());
        Ok(())
    })
}

fn model_info_result(instance: &ModelInfoInstance) -> HelperResult {
    let mut logging = state::state(&instance.details)?;
    let agent_name = get_a2a_model_info(
        instance.card.as_ref(),
        instance.logging.then_some(&mut logging),
    );
    if !instance.logging {
        return Ok(serde_json::json!({
            "agent_name": agent_name,
            "model": serde_json::Value::Null,
            "custom_llm_provider": serde_json::Value::Null,
            "details": serde_json::Value::Null,
        }));
    }
    Ok(serde_json::json!({
        "agent_name": agent_name,
        "model": logging.model,
        "custom_llm_provider": logging.custom_llm_provider,
        "details": state::details_value(&logging)?,
    }))
}

fn build_streaming_result(instance: &BuildStreamingInstance) -> HelperResult {
    let mut context = CorrelationContext::default();
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)
        .map_err(|error| helper_error("RedactorError", error.to_string()))?;
    let request = StreamingLoggingRequest {
        request_id: &instance.request.id,
        agent_name: &instance.agent_name,
        agent_id: instance.agent_id.as_deref(),
        litellm_params: instance.litellm_params.as_ref(),
        metadata: instance.metadata.as_ref(),
        proxy_server_request: instance.proxy_server_request.as_ref(),
    };
    let logging = build_streaming_logging_obj(
        &request,
        LoggingDeps {
            start_time: state::fixed_time(state::FIXED_START)?,
            correlation: CorrelationSetup {
                context: &mut context,
                settings: CorrelationSettings::default(),
            },
            redactor: &redactor,
            generated_trace_id: "generated-trace",
            clock: || Duration::ZERO,
        },
    )
    .map_err(|error| helper_error("LoggingContextError", error.to_string()))?;
    let trace_generated = !logging.litellm_trace_id.is_empty();
    let details = state::details_value(&logging)?;
    Ok(serde_json::json!({
        "model": logging.model,
        "custom_llm_provider": logging.custom_llm_provider,
        "call_type": logging.call_type,
        "stream": logging.stream,
        "litellm_call_id": logging.litellm_call_id,
        "function_id": logging.function_id,
        "messages": logging.messages,
        "litellm_params": logging.litellm_params,
        "optional_params": logging.optional_params,
        "details": details,
        "trace_generated": trace_generated,
    }))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::SetUsage { instances } => run_all(instances, usage_result),
            Input::SetAgentId { instances } => run_all(instances, agent_id_result),
            Input::CostParams { instances } => run_all(instances, |instance| {
                Ok(
                    serde_json::json!({ "cost_params": a2a_cost_params(instance.litellm_params.as_ref()) }),
                )
            }),
            Input::ModelInfo { instances } => run_all(instances, model_info_result),
            Input::BuildStreaming { instances } => run_all(instances, build_streaming_result),
            Input::Iterate { instances } => streaming::run(instances),
        }
    }
}
