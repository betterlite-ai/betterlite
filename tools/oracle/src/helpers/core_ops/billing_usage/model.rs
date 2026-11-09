use std::str::FromStr as _;

use betterlite_core::cost::usage::BillingUsage;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_models::cost_usage::transcription::TranscriptionUsage;
use betterlite_models::decisions::Json;
use betterlite_models::decisions::openai::DecisionUsage;
use betterlite_models::decisions::systemone::DecisionsUsage;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::{Map, Value};

use super::{Call, Failure, UsageKindInput, UsageSpec, failure};
pub(super) enum OwnedUsage {
    Absent,
    Null,
    Chat(Usage),
    Response(ResponseApiUsage),
    Transcription(TranscriptionUsage),
    DecisionsIr(DecisionsUsage),
    DecisionsOpenAi(DecisionUsage),
    Interactions(IndexMap<String, Json>),
    Mapping(IndexMap<String, Json>),
}
impl OwnedUsage {
    pub(super) const fn borrowed(&self) -> BillingUsage<'_> {
        match self {
            Self::Absent => BillingUsage::Absent,
            Self::Null => BillingUsage::Null,
            Self::Chat(usage) => BillingUsage::Chat(usage),
            Self::Response(usage) => BillingUsage::Response(usage),
            Self::Transcription(usage) => BillingUsage::Transcription(usage),
            Self::DecisionsIr(usage) => BillingUsage::DecisionsIr(usage),
            Self::DecisionsOpenAi(usage) => BillingUsage::DecisionsOpenAi(usage),
            Self::Interactions(mapping) => BillingUsage::Interactions(mapping),
            Self::Mapping(mapping) => BillingUsage::Mapping(mapping),
        }
    }
}
pub(super) fn parse_usage(spec: Option<&UsageSpec>) -> Result<OwnedUsage, Failure> {
    let Some(found) = spec else {
        return Ok(OwnedUsage::Absent);
    };
    let value = || {
        found
            .value
            .clone()
            .ok_or_else(|| failure("value is required"))
    };
    match found.kind {
        UsageKindInput::Absent => Ok(OwnedUsage::Absent),
        UsageKindInput::Null => Ok(OwnedUsage::Null),
        UsageKindInput::Chat => usage_from_mapping(&mapping_value(value()?)?)
            .map(OwnedUsage::Chat)
            .map_err(failure),
        UsageKindInput::Responses => Ok(OwnedUsage::Response(
            serde_json::from_value(value()?).map_err(failure)?,
        )),
        UsageKindInput::Transcription => Ok(OwnedUsage::Transcription(
            serde_json::from_value(value()?).map_err(failure)?,
        )),
        UsageKindInput::DecisionsIr => Ok(OwnedUsage::DecisionsIr(
            serde_json::from_value(value()?).map_err(failure)?,
        )),
        UsageKindInput::DecisionsOpenAi => Ok(OwnedUsage::DecisionsOpenAi(
            serde_json::from_value(value()?).map_err(failure)?,
        )),
        UsageKindInput::Interactions => Ok(OwnedUsage::Interactions(mapping_value(value()?)?)),
        UsageKindInput::Mapping => Ok(OwnedUsage::Mapping(mapping_value(value()?)?)),
    }
}
fn mapping_value(value: Value) -> Result<IndexMap<String, Json>, Failure> {
    let parsed: Json = serde_json::from_value(value).map_err(failure)?;
    let Json::Object(fields) = parsed else {
        return Err(failure("the usage mapping must be an object"));
    };
    Ok(fields)
}
pub(super) fn optional_mapping(
    value: Option<&Value>,
) -> Result<Option<IndexMap<String, Json>>, Failure> {
    let Some(found) = value else {
        return Ok(None);
    };
    mapping_value(found.clone()).map(Some)
}
pub(super) fn optional_usage(call: &Call) -> Result<Option<Usage>, Failure> {
    let Some(value) = call.usage.as_ref() else {
        return Ok(None);
    };
    let mapping = mapping_value(value.clone())?;

    usage_from_mapping(&mapping).map(Some).map_err(failure)
}
pub(super) fn usage_projection(usage: Option<&Usage>) -> Value {
    let Some(found_usage) = usage else {
        return Value::Null;
    };
    let mut record = Map::new();
    for (name, count) in [
        ("prompt_tokens", found_usage.prompt_tokens),
        ("completion_tokens", found_usage.completion_tokens),
        ("total_tokens", found_usage.total_tokens),
    ] {
        record.insert(name.to_owned(), Value::from(count));
    }
    if let Some(details) = found_usage.prompt_tokens_details.as_ref() {
        record.insert(
            "prompt_tokens_details".to_owned(),
            details_projection(details, &PROMPT_FIELDS),
        );
    }
    if let Some(details) = found_usage.completion_tokens_details.as_ref() {
        record.insert(
            "completion_tokens_details".to_owned(),
            details_projection(details, &COMPLETION_FIELDS),
        );
    }
    if let Some(cost) = found_usage.cost {
        record.insert("cost".to_owned(), decimal_value(&cost));
    }
    record.insert(
        "extras".to_owned(),
        serde_json::to_value(&found_usage.extras).unwrap_or(Value::Null),
    );
    Value::Object(record)
}
const PROMPT_FIELDS: [&str; 6] = [
    "text_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
    "cached_tokens",
    "cache_write_tokens",
];
const COMPLETION_FIELDS: [&str; 5] = [
    "text_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
    "reasoning_tokens",
];
fn details_projection(details: &IndexMap<String, Json>, fields: &[&str]) -> Value {
    let mut record = Map::new();
    for name in fields {
        if let Some(value) = details.get(*name) {
            record.insert(
                (*name).to_owned(),
                serde_json::to_value(value).unwrap_or(Value::Null),
            );
        }
    }
    Value::Object(record)
}
pub(super) fn decimal_value(cost: &Decimal) -> Value {
    let text = cost.normalize().to_string();
    serde_json::Number::from_str(&text)
        .map_or_else(|_error| Value::String(cost.to_string()), Value::Number)
}
