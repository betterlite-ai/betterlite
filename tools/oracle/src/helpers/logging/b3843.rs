use betterlite_logging::otel::model::payloads::guardrail::GuardrailSpanData;
use betterlite_logging::otel::model::payloads::{
    cost::LLMCost,
    request_params::LLMRequestParams,
    server::ServerInfo,
    service::{ServiceLoggerPayload, ServiceSpanData, ServiceType},
    usage::LLMUsage,
};
use betterlite_logging::otel::model::payloads::{helpers, sanitize_event_metadata};
use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
use serde_json::Value;
use std::collections::BTreeMap;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let function = input
            .get("function")
            .and_then(Value::as_str)
            .unwrap_or_default();
        dispatch(function, input)
    }
}

fn dispatch(function: &str, input: &Value) -> crate::helpers::HelperResult {
    match function {
        "sanitize_event_metadata" => run_sanitize(input),
        "guardrail_error_status" => run_guardrail(input),
        "request_params" | "usage" | "cost" | "server_info" | "service" => {
            run_payload(function, input)
        }
        "cache_token_value"
        | "guardrail_mode_str"
        | "total_masked_entities"
        | "dicts"
        | "json_or_none" => run_value_helper(function, input),
        _ => Err(invalid(&format!("unknown B3843 function {function}"))),
    }
}

fn run_payload(function: &str, input: &Value) -> crate::helpers::HelperResult {
    match function {
        "server_info" => {
            let info = ServerInfo::from_api_base(input.get("value").and_then(Value::as_str));
            Ok(info.map_or(
                Value::Null,
                |server| serde_json::json!({"address":server.address,"port":server.port}),
            ))
        }
        "request_params" => run_request_params(input),
        "cost" => run_cost(input),
        "usage" => run_usage(input),
        "service" => run_service(input),
        _ => Err(invalid("unsupported payload factory")),
    }
}

fn run_request_params(input: &Value) -> crate::helpers::HelperResult {
    let params = parse_fields(input, "value", "request parameters must be an object")?;
    let parsed = LLMRequestParams::from_model_parameters(&params);
    Ok(
        serde_json::json!({"temperature":parsed.temperature,"top_p":parsed.top_p,"top_k":parsed.top_k,"max_tokens":parsed.max_tokens,"frequency_penalty":parsed.frequency_penalty,"presence_penalty":parsed.presence_penalty,"stop_sequences":parsed.stop_sequences,"seed":parsed.seed}),
    )
}

fn run_usage(input: &Value) -> crate::helpers::HelperResult {
    let values = parse_fields(input, "value", "usage payload must be an object")?;
    let parsed = LLMUsage::from_payload(&values);
    Ok(
        serde_json::json!({"input_tokens":parsed.input_tokens,"output_tokens":parsed.output_tokens,"total_tokens":parsed.total_tokens,"cache_creation_input_tokens":parsed.cache_creation_input_tokens,"cache_read_input_tokens":parsed.cache_read_input_tokens,"reasoning_tokens":parsed.reasoning_tokens,"uncached_input_tokens":parsed.uncached_input_tokens(),"non_reasoning_output_tokens":parsed.non_reasoning_output_tokens()}),
    )
}

fn run_cost(input: &Value) -> crate::helpers::HelperResult {
    let values = parse_fields(input, "value", "cost breakdown must be an object")?;
    let parsed = LLMCost::from_breakdown(&values);
    let to_float = |amount: Option<rust_decimal::Decimal>| {
        amount
            .map(|decimal| decimal.to_string().parse::<f64>())
            .transpose()
            .ok()
            .flatten()
    };
    Ok(
        serde_json::json!({"input":to_float(parsed.input),"output":to_float(parsed.output),"cache_read":to_float(parsed.cache_read),"cache_creation":to_float(parsed.cache_creation),"tool_usage":to_float(parsed.tool_usage),"original":to_float(parsed.original),"discount_amount":to_float(parsed.discount_amount),"discount_percent":to_float(parsed.discount_percent),"margin_fixed_amount":to_float(parsed.margin_fixed_amount),"margin_percent":to_float(parsed.margin_percent),"margin_total_amount":to_float(parsed.margin_total_amount)}),
    )
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

fn run_service(input: &Value) -> crate::helpers::HelperResult {
    let raw = input
        .get("value")
        .and_then(Value::as_object)
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
    let metadata = parse_fields(
        input.get("value").unwrap_or(&Value::Null),
        "event_metadata",
        "event metadata must be an object",
    )?;
    let parsed = ServiceSpanData::from_payload(&payload, &metadata);
    Ok(
        serde_json::json!({"service_name":parsed.service_name,"call_type":parsed.call_type,"caller":parsed.caller,"error":parsed.error.map(|error|error.message),"event_metadata":parsed.event_metadata}),
    )
}

fn run_value_helper(function: &str, input: &Value) -> crate::helpers::HelperResult {
    let raw_value = input.get("value").unwrap_or(&Value::Null);
    let parsed_value = parse_model_value(raw_value)?;
    let result = match function {
        "cache_token_value" => {
            let values = input
                .get("values")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(parse_model_value)
                .collect::<Result<Vec<_>, _>>()?;
            helpers::cache_token_value(&values).map_or(Value::Null, Value::from)
        }
        "guardrail_mode_str" => {
            helpers::guardrail_mode_str(&parsed_value).map_or(Value::Null, Value::String)
        }
        "total_masked_entities" => {
            helpers::total_masked_entities(&parsed_value).map_or(Value::Null, Value::from)
        }
        "dicts" => serde_json::json!(helpers::dicts(&parsed_value)),
        "json_or_none" => helpers::json_or_none(&parsed_value).map_or(Value::Null, Value::String),
        _ => return Err(invalid("unsupported B3843 value helper")),
    };
    Ok(result)
}

fn run_sanitize(input: &Value) -> crate::helpers::HelperResult {
    let fields = parse_fields(input, "value", "event metadata must be an object")?;
    Ok(serde_json::json!(
        sanitize_event_metadata::sanitize_event_metadata(&fields)
    ))
}

fn run_guardrail(input: &Value) -> crate::helpers::HelperResult {
    let fields = parse_fields(input, "entry", "guardrail entry must be an object")?;
    let parsed = GuardrailSpanData::from_logging_entry(&fields);
    Ok(
        serde_json::json!({"status": parsed.status, "error_type": parsed.error.and_then(|error| error.error_type)}),
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

mod tests;
