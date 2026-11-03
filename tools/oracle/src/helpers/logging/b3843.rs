mod input;
mod tests;

use betterlite_logging::otel::model::payloads::{
    cost::LLMCost,
    error::SpanError,
    guardrail::GuardrailSpanData,
    helpers,
    request_params::LLMRequestParams,
    sanitize_event_metadata::sanitize_event_metadata,
    server::ServerInfo,
    service::{ServiceLoggerPayload, ServiceSpanData, ServiceType},
    usage::LLMUsage,
};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use input::Input;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::RequestParams { value } => json!(LLMRequestParams::from_model_parameters(value)),
            Input::Cost { value } => json!(LLMCost::from_breakdown(value)),
            Input::ServerInfo { value } => json!(ServerInfo::from_api_base(value.as_deref())),
            Input::Usage { value } => {
                let parsed = LLMUsage::from_payload(value);
                json!({"input_tokens":parsed.input_tokens,"output_tokens":parsed.output_tokens,"total_tokens":parsed.total_tokens,"cache_creation_input_tokens":parsed.cache_creation_input_tokens,"cache_read_input_tokens":parsed.cache_read_input_tokens,"reasoning_tokens":parsed.reasoning_tokens,"uncached_input_tokens":parsed.uncached_input_tokens(),"non_reasoning_output_tokens":parsed.non_reasoning_output_tokens()})
            }
            Input::Service { value } => run_service(value)?,
            Input::SpanError { value } => json!(SpanError::from_payload(value)),
            Input::ProxyRequest { value } => json!(value),
            Input::Guardrail { entry } => json!(GuardrailSpanData::from_logging_entry(entry)),
            Input::GuardrailErrorStatus { entry } => {
                let parsed = GuardrailSpanData::from_logging_entry(entry);
                json!({"status":parsed.status,"error_type":parsed.error.and_then(|error| error.error_type)})
            }
            Input::CacheTokenValue { values } => json!(helpers::cache_token_value(values)),
            Input::GuardrailModeStr { value } => json!(helpers::guardrail_mode_str(value)),
            Input::TotalMaskedEntities { value } => json!(helpers::total_masked_entities(value)),
            Input::Dicts { value } => json!(helpers::dicts(value)),
            Input::JsonOrNone { value } => json!(helpers::json_or_none(value)),
            Input::SanitizeEventMetadata { value } => json!(sanitize_event_metadata(value)),
        };
        Ok(json!({"result":result}))
    }
}

fn service_type(name: &str) -> Option<ServiceType> {
    Some(match name {
        "redis" => ServiceType::Redis,
        "postgres" => ServiceType::Postgres,
        "batch_write_to_db" => ServiceType::BatchWriteToDb,
        "reset_budget_job" => ServiceType::ResetBudgetJob,
        "self" => ServiceType::SelfService,
        "router" => ServiceType::Router,
        "auth" => ServiceType::Auth,
        "proxy_pre_call" => ServiceType::ProxyPreCall,
        "pod_lock_manager" => ServiceType::PodLockManager,
        "in_memory_daily_spend_update_queue" => ServiceType::InMemoryDailySpendUpdateQueue,
        "redis_daily_spend_update_queue" => ServiceType::RedisDailySpendUpdateQueue,
        "redis_daily_end_user_spend_update_queue" => ServiceType::RedisDailyEndUserQueue,
        "redis_daily_org_spend_update_queue" => ServiceType::RedisDailyOrgSpendUpdateQueue,
        "redis_daily_team_spend_update_queue" => ServiceType::RedisDailyTeamSpendUpdateQueue,
        "redis_daily_agent_spend_update_queue" => ServiceType::RedisDailyAgentQueue,
        "redis_daily_tag_spend_update_queue" => ServiceType::RedisDailyTagSpendUpdateQueue,
        "in_memory_spend_update_queue" => ServiceType::InMemorySpendUpdateQueue,
        "redis_spend_update_queue" => ServiceType::RedisSpendUpdateQueue,
        "redis_window_spend_update_queue" => ServiceType::RedisWindowSpendUpdateQueue,
        _ => return None,
    })
}

fn run_service(value: &Value) -> crate::helpers::HelperResult {
    let raw = value
        .as_object()
        .ok_or_else(|| invalid("service payload must be an object"))?;
    let name = raw
        .get("service")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let service = service_type(name).ok_or_else(|| invalid("unknown service type"))?;
    let payload = ServiceLoggerPayload {
        service,
        call_type: raw
            .get("call_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        caller: raw.get("caller").and_then(Value::as_str).map(str::to_owned),
        error: raw.get("error").and_then(Value::as_str).map(str::to_owned),
    };
    let metadata = parse_fields(value, "event_metadata", "event metadata must be an object")?;
    let parsed = ServiceSpanData::from_payload(&payload, &metadata);
    Ok(
        serde_json::json!({"service_name":parsed.service_name,"call_type":parsed.call_type,"caller":parsed.caller,"error":parsed.error.map(|error|error.message),"event_metadata":parsed.event_metadata}),
    )
}

fn parse_fields(
    input: &Value,
    field: &str,
    error_message: &str,
) -> Result<BTreeMap<String, OtelModelValue>, Box<crate::helpers::HelperError>> {
    let raw_fields = input
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| invalid(error_message))?;
    raw_fields
        .iter()
        .map(|(key, raw)| parse_model_value(raw).map(|parsed| (key.clone(), parsed)))
        .collect()
}

fn parse_model_value(value: &Value) -> Result<OtelModelValue, Box<crate::helpers::HelperError>> {
    match serde_json::from_value(value.clone()) {
        Ok(parsed) => Ok(parsed),
        Err(error) => Err(invalid(&error.to_string())),
    }
}

fn invalid(message: &str) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
