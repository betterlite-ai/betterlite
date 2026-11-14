use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use crate::fields::timestamps::with_fraction;
use crate::fields::usage::Usage;
use crate::fields::usage::parse::usage_from_mapping;
use crate::langfuse_projections::cache_usage::cache_read_tokens;

use super::AssemblyError;

// the response object's usage mapping, which is absent when the response carries none or an empty one
pub fn from_response(response: Option<&Json>) -> Result<Option<Usage>, AssemblyError> {
    let Some(Json::Object(fields)) = response else {
        return Ok(None);
    };
    let Some(Json::Object(usage)) = fields.get("usage") else {
        return Ok(None);
    };
    if usage.is_empty() {
        return Ok(None);
    }
    usage_from_mapping(usage)
        .map(Some)
        .map_err(AssemblyError::Usage)
}

// litellm.types.integrations.langfuse.LangfuseUsageDetails, with the input reduced by the cache reads
#[must_use]
pub fn details(usage: &Usage) -> Json {
    let cache_read = cache_read_tokens(usage);
    let cache_creation = cache_creation_tokens(usage);
    let mut record = IndexMap::new();
    record.insert(
        "input".to_owned(),
        Json::Number(Number::from(usage.prompt_tokens.saturating_sub(cache_read))),
    );
    record.insert(
        "output".to_owned(),
        Json::Number(Number::from(usage.completion_tokens)),
    );
    record.insert(
        "total".to_owned(),
        Json::Number(Number::from(usage.total_tokens)),
    );
    record.insert(
        "cache_creation_input_tokens".to_owned(),
        Json::Number(Number::from(cache_creation)),
    );
    record.insert(
        "cache_read_input_tokens".to_owned(),
        Json::Number(Number::from(cache_read)),
    );
    Json::Object(record)
}

// LiteLLM reads the token count off the usage object and writes 0 for a null
fn cache_creation_tokens(usage: &Usage) -> i64 {
    let Some(Json::Number(number)) = usage.extras.get("cache_creation_input_tokens") else {
        return 0;
    };
    let Ok(amount) = Decimal::from_str(&number.to_string()) else {
        return 0;
    };
    amount.to_i64().unwrap_or(0)
}

// cost_details carries the total only when a usage and a numeric response cost both exist
#[must_use]
pub fn cost_details(usage: Option<&Usage>, cost: Option<Decimal>) -> Option<Json> {
    match (usage, cost) {
        (Some(_usage), Some(total)) => {
            let mut record = IndexMap::new();
            record.insert("total".to_owned(), cost_value(total));
            Some(Json::Object(record))
        }
        (None, _) | (Some(_), None) => None,
    }
}

#[must_use]
pub fn cost_value(cost: Decimal) -> Json {
    match Number::from_str(&with_fraction(cost).to_string()) {
        Ok(number) => Json::Number(number),
        Err(_error) => Json::Null,
    }
}
