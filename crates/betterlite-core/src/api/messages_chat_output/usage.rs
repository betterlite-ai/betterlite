use betterlite_models::cost_usage::server_tool_projection::{
    ServerToolRecord, web_search_requests,
};
use betterlite_models::decisions::Json as ModelJson;
use betterlite_prompts::arguments::json::Json;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use crate::api::messages_chat_output::output::{ServerToolProjection, UsageProjection};
use crate::api::messages_chat_output::records::UsageRecord;

const CACHE_READ_DETAILS: [&str; 1] = ["cached_tokens"];
const CACHE_CREATION_DETAILS: [&str; 2] = ["cache_creation_tokens", "cache_write_tokens"];
const WEB_SEARCH_DETAILS: [&str; 1] = ["web_search_requests"];

// _positive_int: a bool is excluded before the integer check, and only an integral float survives.
fn positive_number(number: &Number) -> i64 {
    if let Some(integer) = number.as_i64() {
        return integer.max(0);
    }
    if let Some(unsigned) = number.as_u64() {
        return i64::try_from(unsigned).unwrap_or(i64::MAX);
    }
    let Some(float) = number.as_f64() else {
        return 0;
    };
    if float.fract() != 0.0_f64 || float <= 0.0_f64 {
        return 0;
    }
    Decimal::try_from(float)
        .ok()
        .and_then(|decimal| decimal.to_i64())
        .unwrap_or(0)
}

fn positive_integer(value: &Json) -> i64 {
    match value {
        Json::Number(number) => positive_number(number),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => 0,
    }
}

fn model_positive_integer(value: &ModelJson) -> i64 {
    match value {
        ModelJson::Number(number) => positive_number(number),
        ModelJson::Null
        | ModelJson::Bool(_)
        | ModelJson::Text(_)
        | ModelJson::List(_)
        | ModelJson::Object(_) => 0,
    }
}

fn first_positive(first: Option<&Json>, second: Option<&Json>) -> i64 {
    first
        .into_iter()
        .chain(second)
        .map(positive_integer)
        .find(|count| *count > 0)
        .unwrap_or(0)
}

fn first_positive_detail(usage: &UsageRecord, keys: &[&str]) -> i64 {
    let Some(details) = &usage.prompt_tokens_details else {
        return 0;
    };
    keys.iter()
        .filter_map(|key| details.get(*key))
        .map(positive_integer)
        .find(|count| *count > 0)
        .unwrap_or(0)
}

fn cache_read_input_tokens(usage: &UsageRecord) -> i64 {
    let explicit = first_positive(
        usage.cache_read_input_tokens.as_ref(),
        usage.private_cache_read_input_tokens.as_ref(),
    );
    if explicit > 0 {
        return explicit;
    }
    first_positive_detail(usage, &CACHE_READ_DETAILS)
}

fn cache_creation_input_tokens(usage: &UsageRecord) -> i64 {
    let explicit = first_positive(
        usage.cache_creation_input_tokens.as_ref(),
        usage.private_cache_creation_input_tokens.as_ref(),
    );
    if explicit > 0 {
        return explicit;
    }
    first_positive_detail(usage, &CACHE_CREATION_DETAILS)
}

fn web_search_request_count(usage: &UsageRecord) -> i64 {
    let from_server_tool_use = usage
        .server_tool_use
        .as_ref()
        .and_then(|mapping| web_search_requests(Some(ServerToolRecord::Mapping(mapping))))
        .map_or(0, |value| model_positive_integer(&value));
    if from_server_tool_use > 0 {
        return from_server_tool_use;
    }
    first_positive_detail(usage, &WEB_SEARCH_DETAILS)
}

#[must_use]
pub fn project_usage(usage: &UsageRecord) -> UsageProjection {
    let cache_read = cache_read_input_tokens(usage);
    let cache_creation = cache_creation_input_tokens(usage);
    let web_search_requests = web_search_request_count(usage);
    let prompt_tokens = usage.prompt_tokens.unwrap_or(0);
    UsageProjection {
        input_tokens: prompt_tokens
            .saturating_sub(cache_read)
            .saturating_sub(cache_creation)
            .max(0),
        output_tokens: usage.completion_tokens.unwrap_or(0),
        cache_creation_input_tokens: (cache_creation > 0).then_some(cache_creation),
        cache_read_input_tokens: (cache_read > 0).then_some(cache_read),
        server_tool_use: (web_search_requests > 0).then_some(ServerToolProjection {
            web_search_requests,
        }),
        iterations: None,
    }
}

#[must_use]
pub fn zero_usage() -> UsageProjection {
    project_usage(&UsageRecord::default())
}
