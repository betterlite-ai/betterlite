use betterlite_models::decisions::Json;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

use super::{GuardrailMetadata, GuardrailSpendLog};

const DATE_FORMAT: &str = "%Y-%m-%d";

// Python's str(datetime) writes a space before the offset, and a spend log read back from the database carries that shape
const OFFSET_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.f%#z";

const NAIVE_FORMATS: [&str; 3] = [
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
pub enum PayloadStartTime {
    Parsed(DateTime<Utc>),
    Text(String),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(transparent)]
pub struct UnitCost(#[serde(with = "rust_decimal::serde::arbitrary_precision")] pub Decimal);

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct GuardrailEntry {
    pub guardrail_id: Option<String>,
    pub guardrail_name: Option<String>,
    pub guardrail_status: Option<String>,
    pub policy_id: Option<String>,
    pub guardrail_usage: Option<IndexMap<String, Json>>,
    pub guardrail_cost_by_unit: Option<IndexMap<String, Option<UnitCost>>>,
    pub guardrail_cost_in_spend: Option<bool>,
}

// LiteLLM's metadata is either the object itself or a JSON string holding it
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(from = "MetadataWire")]
pub struct PayloadMetadata(pub GuardrailMetadata);

#[derive(Deserialize)]
#[serde(untagged)]
enum MetadataWire {
    Inline(GuardrailMetadata),
    Text(String),
}

impl From<MetadataWire> for PayloadMetadata {
    fn from(wire: MetadataWire) -> Self {
        match wire {
            MetadataWire::Inline(metadata) => Self(metadata),
            MetadataWire::Text(text) => Self(
                serde_json::from_str::<GuardrailMetadata>(&text).unwrap_or_else(|error| {
                    tracing::warn!("guardrail usage: metadata text is not a JSON object: {error}");
                    GuardrailMetadata::default()
                }),
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadStamp {
    pub request_id: String,
    pub date: String,
    pub instant: DateTime<Utc>,
}

#[must_use]
pub fn payload_stamp(payload: &GuardrailSpendLog) -> Option<PayloadStamp> {
    let request_id = payload.request_id.as_deref().filter(|id| !id.is_empty())?;
    let instant = start_instant(payload)?;
    Some(PayloadStamp {
        request_id: request_id.to_owned(),
        date: instant.format(DATE_FORMAT).to_string(),
        instant,
    })
}

#[must_use]
pub fn information_entries(payload: &GuardrailSpendLog) -> &[GuardrailEntry] {
    let Some(metadata) = payload.metadata.as_ref().map(|wrapped| &wrapped.0) else {
        return &[];
    };
    let primary = metadata
        .guardrail_information
        .as_deref()
        .unwrap_or_default();
    if primary.is_empty() {
        metadata.standard_logging.as_deref().unwrap_or_default()
    } else {
        primary
    }
}

#[must_use]
pub(crate) fn ids_by_name(entries: &[GuardrailEntry]) -> IndexMap<String, String> {
    let mut ids = IndexMap::new();
    for entry in entries {
        let id = entry.guardrail_id.as_deref().filter(|id| !id.is_empty());
        let name = entry
            .guardrail_name
            .as_deref()
            .filter(|name| !name.is_empty());
        if let (Some(id), Some(name)) = (id, name) {
            ids.insert(name.to_owned(), id.to_owned());
        }
    }
    ids
}

#[must_use]
pub(crate) fn resolved_id(
    entry: &GuardrailEntry,
    ids: &IndexMap<String, String>,
) -> Option<String> {
    let own = entry.guardrail_id.as_deref().filter(|id| !id.is_empty());
    if let Some(id) = own {
        return Some(id.to_owned());
    }
    let name = entry
        .guardrail_name
        .as_deref()
        .filter(|name| !name.is_empty());
    if let Some(id) = name.and_then(|name| ids.get(name)) {
        return Some(id.clone());
    }
    name.map(str::to_owned)
}

// LiteLLM's unit rollup skips the name-to-id map the metrics and index rows read
#[must_use]
pub(crate) fn unit_guardrail_id(entry: &GuardrailEntry) -> Option<String> {
    let own = entry.guardrail_id.as_deref().filter(|id| !id.is_empty());
    let name = entry
        .guardrail_name
        .as_deref()
        .filter(|name| !name.is_empty());
    own.or(name).map(str::to_owned)
}

// LiteLLM counts a unit only when its value is a positive integer that is not a bool
#[must_use]
pub(crate) fn unit_count(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64().filter(|units| *units > 0),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// billed_guardrail_cost_by_unit: a report-only entry, one with a negative price, and a unit with no price are all unpriced
#[must_use]
pub(crate) fn billed_cost(entry: &GuardrailEntry, unit: &str) -> Option<Decimal> {
    if entry.guardrail_cost_in_spend == Some(false) {
        return None;
    }
    let costs = entry.guardrail_cost_by_unit.as_ref()?;
    if costs
        .values()
        .flatten()
        .any(|cost| cost.0.is_sign_negative())
    {
        return None;
    }
    costs.get(unit).copied().flatten().map(|cost| cost.0)
}

fn start_instant(payload: &GuardrailSpendLog) -> Option<DateTime<Utc>> {
    match payload.start_time.as_ref()? {
        PayloadStartTime::Parsed(instant) => Some(*instant),
        PayloadStartTime::Text(text) => start_from_text(text),
    }
}

fn start_from_text(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(instant) = DateTime::parse_from_rfc3339(text) {
        return Some(instant.with_timezone(&Utc));
    }
    if let Ok(instant) = DateTime::parse_from_str(text, OFFSET_FORMAT) {
        return Some(instant.with_timezone(&Utc));
    }
    for format in NAIVE_FORMATS {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return Some(naive.and_utc());
        }
    }
    NaiveDate::parse_from_str(text, DATE_FORMAT)
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(NaiveDateTime::and_utc)
}
