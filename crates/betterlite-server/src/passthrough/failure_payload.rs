use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_metadata_safety::metadata::metadata_name_from_data;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::fields::usage::project::usage_json;
use betterlite_logging::passthrough_logging_state::usage::UpstreamReportedUsage;
use betterlite_models::call_helpers::metadata::{MetadataSlots, get_or_create_metadata_bucket};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use super::request_context::{PassThroughKwargs, PassthroughLoggingObj, as_object};

const GUARDRAIL_INFO_KEY: &str = "standard_logging_guardrail_information";
const METADATA_KEY: &str = "metadata";
const LITELLM_METADATA_KEY: &str = "litellm_metadata";
const MODEL_KEY: &str = "model";
const PROVIDER_KEY: &str = "custom_llm_provider";
const LOGGING_OBJ_KEY: &str = "litellm_logging_obj";
const RESPONSE_COST_KEY: &str = "response_cost";
const COMBINED_USAGE_KEY: &str = "combined_usage_object";

// _carry_guardrail_logging_info: a blocked post-call guardrail's entries land on the failure payload
pub fn carry_guardrail_logging_info(
    request_data: &mut RequestData,
    guardrail_data: Option<&RequestData>,
) {
    let Some(guardrail) = guardrail_data else {
        return;
    };
    let source = metadata_name_from_data(guardrail).as_str();
    let entries = match guardrail
        .get(source)
        .and_then(as_object)
        .and_then(|bucket| bucket.get(GUARDRAIL_INFO_KEY))
    {
        Some(Json::List(items)) if !items.is_empty() => items.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_))
        | None => return,
    };
    let mut slots = MetadataSlots {
        metadata: request_data.get(METADATA_KEY).and_then(as_object).cloned(),
        litellm_metadata: request_data
            .get(LITELLM_METADATA_KEY)
            .and_then(as_object)
            .cloned(),
    };
    let (name, bucket) = get_or_create_metadata_bucket(&mut slots);
    bucket
        .entry(GUARDRAIL_INFO_KEY.to_owned())
        .or_insert(Json::List(entries));
    request_data.insert(name.key().to_owned(), Json::Object(bucket.clone()));
}

// _build_passthrough_failure_request_payload: one request_data for both failure paths to report with
#[must_use]
pub fn build_failure_payload(
    parsed_body: Option<&RequestData>,
    kwargs: Option<&PassThroughKwargs>,
    logging_obj: Option<&PassthroughLoggingObj>,
    custom_llm_provider: Option<&str>,
    upstream_usage: Option<&UpstreamReportedUsage>,
) -> RequestData {
    let mut request_payload = parsed_body.cloned().unwrap_or_default();
    if let Some(kwargs) = kwargs {
        request_payload.extend(kwargs.to_data());
    }
    if let Some(logging_obj) = logging_obj {
        request_payload.insert(LOGGING_OBJ_KEY.to_owned(), logging_obj.to_json());
    }
    merge_model_and_provider(&mut request_payload, parsed_body, custom_llm_provider);
    if let Some(usage) = upstream_usage {
        insert_usage_keys(&mut request_payload, usage);
    }
    request_payload
}

// model is copied from the body only when kwargs left it out, and a provider is never blank
fn merge_model_and_provider(
    request_payload: &mut RequestData,
    parsed_body: Option<&RequestData>,
    custom_llm_provider: Option<&str>,
) {
    if !request_payload.contains_key(MODEL_KEY)
        && let Some(body) = parsed_body.filter(|body| !body.is_empty())
    {
        request_payload.insert(
            MODEL_KEY.to_owned(),
            body.get(MODEL_KEY)
                .cloned()
                .unwrap_or(Json::Text(String::new())),
        );
    }
    if !request_payload.contains_key(PROVIDER_KEY)
        && let Some(provider) = custom_llm_provider.filter(|provider| !provider.is_empty())
    {
        request_payload.insert(PROVIDER_KEY.to_owned(), Json::Text(provider.to_owned()));
    }
}

// spend tracking attributes a recovered cost only next to a usage object, so both keys go together
fn insert_usage_keys(request_payload: &mut RequestData, usage: &UpstreamReportedUsage) {
    // response_cost or 0.0; a NaN is not a JSON number, so it reports zero too
    let amount = usage.response_cost.filter(|amount| amount.is_finite());
    request_payload.insert(
        RESPONSE_COST_KEY.to_owned(),
        Number::from_f64(amount.unwrap_or(0.0_f64)).map_or(Json::Null, Json::Number),
    );
    request_payload.insert(
        COMBINED_USAGE_KEY.to_owned(),
        usage_json(&combined_usage(usage.total_tokens)),
    );
}

// Usage(total_tokens=upstream_usage.total_tokens or 0) in betterlite's canonical Usage shape
fn combined_usage(total_tokens: Option<i64>) -> Usage {
    Usage {
        prompt_tokens: 0_i64,
        completion_tokens: 0_i64,
        total_tokens: total_tokens.unwrap_or(0_i64),
        prompt_tokens_details: None,
        completion_tokens_details: None,
        server_tool_use: None,
        cost: None,
        extras: IndexMap::new(),
    }
}
