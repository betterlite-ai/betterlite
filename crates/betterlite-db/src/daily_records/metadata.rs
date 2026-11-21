use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::savings_evidence::cache_extraction::UsageObject;
use betterlite_models::savings_evidence::dynamic::MappingOrScalar;
use betterlite_models::savings_evidence::metadata::Metadata;
use betterlite_models::savings_evidence::snapshot::CostBreakdown;
use betterlite_models::savings_evidence::usage::RawUsageObject;
use betterlite_models::savings_pricing::autorouter::AutoRouterDecision;
use betterlite_models::tiered_pricing::Scalar;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::autorouter_facts::{INTERNAL_CALL_ORIGIN_KEY, counts_as_present};
use crate::redis_spend::codec::MAX_SPEND_BUFFER_BYTES;

const STATUS_KEY: &str = "status";
const FAILURE_STATUS: &str = "failure";
const USAGE_OBJECT_KEY: &str = "usage_object";
const COST_BREAKDOWN_KEY: &str = "cost_breakdown";

#[derive(Debug, thiserror::Error)]
pub enum DailyMetadataError {
    #[error("daily metadata exceeds {MAX_SPEND_BUFFER_BYTES} bytes")]
    TooLarge,
    #[error("daily metadata is not a JSON object")]
    NotObject,
    #[error("daily metadata is not valid JSON")]
    Json(#[source] serde_json::Error),
    #[error("the metadata field {field} is not a JSON object")]
    NotAnObject { field: &'static str },
    #[error("the metadata field {field} could not be read")]
    Field {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
}

// Literal["success", "failure"] of get_request_status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Success,
    Failure,
}

// Everything _common_add_spend_log_transaction_to_daily_transaction reads out of payload["metadata"].
#[derive(Debug)]
pub struct DailyMetadata {
    pub savings: Metadata,
    pub internal_call: bool,
    pub usage_object: Option<UsageObject>,
    pub raw_usage: Option<RawUsageObject>,
    pub routing_decision: Option<AutoRouterDecision>,
    pub cost_breakdown: Option<CostBreakdown>,
}

#[derive(Debug, Deserialize)]
struct DailyMetadataDetails {
    internal_call_origin: Option<Json>,
    usage_object: Option<Json>,
    routing_decision: Option<MappingOrScalar<AutoRouterDecision>>,
    cost_breakdown: Option<Json>,
}

// The source json.loads the logged metadata before it derives any field, so a malformed value fails the row the same way.
pub fn decode_daily_metadata(bytes: &[u8]) -> Result<DailyMetadata, DailyMetadataError> {
    if bytes.len() > MAX_SPEND_BUFFER_BYTES {
        return Err(DailyMetadataError::TooLarge);
    }
    if bytes.trim_ascii_start().first() != Some(&b'{') {
        return Err(DailyMetadataError::NotObject);
    }
    let savings: Metadata = serde_json::from_slice(bytes).map_err(DailyMetadataError::Json)?;
    let details: DailyMetadataDetails =
        serde_json::from_slice(bytes).map_err(DailyMetadataError::Json)?;
    Ok(DailyMetadata {
        savings,
        internal_call: details
            .internal_call_origin
            .as_ref()
            .is_some_and(counts_as_present),
        usage_object: metadata_object(details.usage_object.as_ref(), USAGE_OBJECT_KEY)?,
        raw_usage: raw_usage_object(details.usage_object.as_ref()),
        routing_decision: details
            .routing_decision
            .as_ref()
            .and_then(MappingOrScalar::as_mapping)
            .cloned(),
        cost_breakdown: metadata_object(details.cost_breakdown.as_ref(), COST_BREAKDOWN_KEY)?,
    })
}

// get_request_status never fails: a metadata value that is absent, unparsable or not a mapping reads as a success.
#[must_use]
pub fn request_status(metadata: Option<&[u8]>) -> RequestStatus {
    let Some(bytes) = metadata else {
        return RequestStatus::Success;
    };
    let Ok(object) = serde_json::from_slice::<BTreeMap<String, Scalar>>(bytes) else {
        return RequestStatus::Success;
    };
    match object.get(STATUS_KEY) {
        Some(Scalar::Text(status)) if status == FAILURE_STATUS => RequestStatus::Failure,
        _ => RequestStatus::Success,
    }
}

// `_metadata.get(key, {}) or {}`: a falsey value reads as absent, and a truthy one must be a mapping.
fn metadata_object<T>(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<T>, DailyMetadataError>
where
    T: DeserializeOwned,
{
    let Some(value) = value.filter(|value| counts_as_present(value)) else {
        return Ok(None);
    };
    let Json::Object(_) = value else {
        return Err(DailyMetadataError::NotAnObject { field });
    };
    decode_text(&encode(value, field)?, field).map(Some)
}

// _usage_from_spend_log swallows every failure, so an unusable usage object only zeroes the auto-router driver.
fn raw_usage_object(value: Option<&Json>) -> Option<RawUsageObject> {
    let value = value.filter(|value| counts_as_present(value))?;
    let text = serde_json::to_string(value).ok()?;
    serde_json::from_str(&text).ok()
}

fn decode_text<T>(bytes: &[u8], field: &'static str) -> Result<T, DailyMetadataError>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|source| DailyMetadataError::Field { field, source })
}

fn encode(value: &Json, field: &'static str) -> Result<String, DailyMetadataError> {
    serde_json::to_string(value).map_err(|source| DailyMetadataError::Field { field, source })
}
