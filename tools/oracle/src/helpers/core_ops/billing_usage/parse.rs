use betterlite_core::cost::usage::BillingUsage;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_models::cost_usage::transcription::TranscriptionUsage;
use betterlite_models::decisions::Json;
use betterlite_models::decisions::openai::DecisionUsage;
use betterlite_models::decisions::systemone::DecisionsUsage;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use indexmap::IndexMap;
use serde_json::Value;

use super::{Call, UsageKindInput, UsageSpec, failure};

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
    TypedRecord(IndexMap<String, Json>),
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
            Self::TypedRecord(mapping) => BillingUsage::TypedRecord(mapping),
        }
    }
}

pub(super) fn parse_usage(
    spec: Option<&UsageSpec>,
) -> Result<OwnedUsage, Box<crate::helpers::HelperError>> {
    let Some(found_spec) = spec else {
        return Ok(OwnedUsage::Absent);
    };
    match found_spec.kind {
        UsageKindInput::Absent => Ok(OwnedUsage::Absent),
        UsageKindInput::Null => Ok(OwnedUsage::Null),
        UsageKindInput::Chat => usage_from_mapping(&mapping_value(required_value(found_spec)?)?)
            .map(OwnedUsage::Chat)
            .map_err(failure),
        UsageKindInput::Responses => Ok(OwnedUsage::Response(
            serde_json::from_value(required_value(found_spec)?).map_err(failure)?,
        )),
        UsageKindInput::Transcription => Ok(OwnedUsage::Transcription(
            serde_json::from_value(required_value(found_spec)?).map_err(failure)?,
        )),
        UsageKindInput::DecisionsIr => Ok(OwnedUsage::DecisionsIr(
            serde_json::from_value(required_value(found_spec)?).map_err(failure)?,
        )),
        UsageKindInput::DecisionsOpenAi => Ok(OwnedUsage::DecisionsOpenAi(
            serde_json::from_value(required_value(found_spec)?).map_err(failure)?,
        )),
        UsageKindInput::Interactions => Ok(OwnedUsage::Interactions(mapping_value(
            required_value(found_spec)?,
        )?)),
        UsageKindInput::Mapping => Ok(OwnedUsage::Mapping(mapping_value(required_value(
            found_spec,
        )?)?)),
        UsageKindInput::TypedRecord => Ok(OwnedUsage::TypedRecord(mapping_value(required_value(
            found_spec,
        )?)?)),
    }
}

fn required_value(spec: &UsageSpec) -> Result<Value, Box<crate::helpers::HelperError>> {
    spec.value
        .clone()
        .ok_or_else(|| failure("value is required"))
}

fn mapping_value(value: Value) -> Result<IndexMap<String, Json>, Box<crate::helpers::HelperError>> {
    let parsed: Json = serde_json::from_value(value).map_err(failure)?;
    let Json::Object(fields) = parsed else {
        return Err(failure("the usage mapping must be an object"));
    };
    Ok(fields)
}

pub(super) fn optional_mapping(
    value: Option<&Value>,
) -> Result<Option<IndexMap<String, Json>>, Box<crate::helpers::HelperError>> {
    match value {
        None => Ok(None),
        Some(found) => mapping_value(found.clone()).map(Some),
    }
}

pub(super) fn optional_usage(
    call: &Call,
) -> Result<Option<Usage>, Box<crate::helpers::HelperError>> {
    let Some(value) = call.usage.as_ref() else {
        return Ok(None);
    };
    let mapping = mapping_value(value.clone())?;

    usage_from_mapping(&mapping).map(Some).map_err(failure)
}
