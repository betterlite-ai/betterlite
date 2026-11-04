use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Value;
use serde_json::value::RawValue;
use thiserror::Error;

use super::records::union::VectorStoreRecord;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VectorStoreRecordKind {
    ToolParams = 0,
    ManagedStore = 1,
    ManagedStoreList = 2,
    UpdateRequest = 3,
    DeleteRequest = 4,
    InfoRequest = 5,
    SearchRequest = 6,
    SearchResponse = 7,
    SearchFailure = 8,
    DiscoveryDataStore = 9,
    DiscoveryEngine = 10,
    CreateRequest = 11,
    CreateResponse = 12,
    IndexCreateRequest = 13,
    ManagedIndex = 14,
    IndexListResponse = 15,
    IndexEndpoints = 16,
    BedrockRequest = 17,
    BedrockResponse = 18,
    BedrockRow = 19,
}

impl TryFrom<u8> for VectorStoreRecordKind {
    type Error = VectorStoreRecordError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::ToolParams),
            1 => Ok(Self::ManagedStore),
            2 => Ok(Self::ManagedStoreList),
            3 => Ok(Self::UpdateRequest),
            4 => Ok(Self::DeleteRequest),
            5 => Ok(Self::InfoRequest),
            6 => Ok(Self::SearchRequest),
            7 => Ok(Self::SearchResponse),
            8 => Ok(Self::SearchFailure),
            9 => Ok(Self::DiscoveryDataStore),
            10 => Ok(Self::DiscoveryEngine),
            11 => Ok(Self::CreateRequest),
            12 => Ok(Self::CreateResponse),
            13 => Ok(Self::IndexCreateRequest),
            14 => Ok(Self::ManagedIndex),
            15 => Ok(Self::IndexListResponse),
            16 => Ok(Self::IndexEndpoints),
            17 => Ok(Self::BedrockRequest),
            18 => Ok(Self::BedrockResponse),
            19 => Ok(Self::BedrockRow),
            _ => Err(VectorStoreRecordError::UnknownKind(value)),
        }
    }
}

#[derive(Debug, Error)]
pub enum VectorStoreRecordError {
    #[error("unknown vector store record kind: {0}")]
    UnknownKind(u8),
    #[error("vector store record JSON exceeds byte limit")]
    JsonTooLarge,
    #[error("vector store record nesting limit exceeded")]
    TooDeep,
    #[error("vector store record item limit exceeded")]
    TooManyItems,
    #[error("invalid vector store record: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Field required: {0}")]
    MissingField(String),
    #[error("invalid literal for field `{field}`: {value}")]
    InvalidLiteral { field: String, value: String },
    #[error("score cannot be represented as an exact decimal")]
    ScoreOutOfRange,
    #[error("invalid UTC timestamp in `{0}`")]
    InvalidTimestamp(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WireDecodeLimits {
    pub max_json_bytes: usize,
    pub max_event_bytes: usize,
    pub max_items: usize,
}

pub fn decode_vector_store_records(
    kind: VectorStoreRecordKind,
    bytes: &[u8],
    limits: WireDecodeLimits,
) -> Result<VectorStoreRecord, VectorStoreRecordError> {
    if bytes.len() > limits.max_json_bytes || bytes.len() > limits.max_event_bytes {
        return Err(VectorStoreRecordError::JsonTooLarge);
    }
    let root: &RawValue = serde_json::from_slice(bytes)?;
    let mut pending = vec![(root, 0_usize)];
    let mut remaining = limits.max_items;
    while let Some((value, depth)) = pending.pop() {
        if depth > 128 {
            return Err(VectorStoreRecordError::TooDeep);
        }
        let children = match value.get().as_bytes().first() {
            Some(b'[') => serde_json::from_str::<Vec<&RawValue>>(value.get())?,
            Some(b'{') => serde_json::from_str::<Members<'_>>(value.get())?
                .0
                .into_iter()
                .map(|(_, child)| child)
                .collect(),
            _ => Vec::new(),
        };
        remaining = remaining
            .checked_sub(children.len())
            .ok_or(VectorStoreRecordError::TooManyItems)?;
        pending.extend(
            children
                .into_iter()
                .map(|child| (child, depth.saturating_add(1))),
        );
    }
    let parsed: Value = serde_json::from_slice(bytes)?;
    if kind == VectorStoreRecordKind::ManagedIndex {
        validate_managed_timestamps(&parsed)?;
    }
    if matches!(
        kind,
        VectorStoreRecordKind::SearchResponse | VectorStoreRecordKind::BedrockResponse
    ) {
        validate_result_scores(kind, &parsed)?;
    }
    VectorStoreRecord::from_value(kind, parsed)
}

fn validate_managed_timestamps(record: &Value) -> Result<(), VectorStoreRecordError> {
    let Some(fields) = record.as_object() else {
        return Ok(());
    };
    for field in ["created_at", "updated_at"] {
        let Some(timestamp) = fields.get(field).and_then(Value::as_str) else {
            continue;
        };
        if !valid_utc_timestamp(timestamp) {
            return Err(VectorStoreRecordError::InvalidTimestamp(field.to_owned()));
        }
    }
    Ok(())
}

fn valid_utc_timestamp(value: &str) -> bool {
    let Some(timestamp) = value.strip_suffix('Z') else {
        return false;
    };
    let Some((date, time)) = timestamp.split_once('T') else {
        return false;
    };
    let mut date_parts = date.split('-');
    let (Some(year), Some(month), Some(day), None) = (
        date_parts.next().and_then(|part| part.parse::<u32>().ok()),
        date_parts.next().and_then(|part| part.parse::<u32>().ok()),
        date_parts.next().and_then(|part| part.parse::<u32>().ok()),
        date_parts.next(),
    ) else {
        return false;
    };
    let mut time_parts = time.split(':');
    let (Some(hour), Some(minute), Some(second), None) = (
        time_parts.next().and_then(|part| part.parse::<u32>().ok()),
        time_parts.next().and_then(|part| part.parse::<u32>().ok()),
        time_parts.next().and_then(|part| part.parse::<u32>().ok()),
        time_parts.next(),
    ) else {
        return false;
    };
    let days = days_in_month(year, month);
    date.len() == 10
        && time.len() == 8
        && day > 0
        && day <= days
        && hour < 24
        && minute < 60
        && second < 60
}

const fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

fn validate_result_scores(
    kind: VectorStoreRecordKind,
    record: &Value,
) -> Result<(), VectorStoreRecordError> {
    let field = match kind {
        VectorStoreRecordKind::SearchResponse => "data",
        VectorStoreRecordKind::BedrockResponse => "retrievalResults",
        VectorStoreRecordKind::ToolParams
        | VectorStoreRecordKind::ManagedStore
        | VectorStoreRecordKind::ManagedStoreList
        | VectorStoreRecordKind::UpdateRequest
        | VectorStoreRecordKind::DeleteRequest
        | VectorStoreRecordKind::InfoRequest
        | VectorStoreRecordKind::SearchRequest
        | VectorStoreRecordKind::SearchFailure
        | VectorStoreRecordKind::DiscoveryDataStore
        | VectorStoreRecordKind::DiscoveryEngine
        | VectorStoreRecordKind::CreateRequest
        | VectorStoreRecordKind::CreateResponse
        | VectorStoreRecordKind::IndexCreateRequest
        | VectorStoreRecordKind::ManagedIndex
        | VectorStoreRecordKind::IndexListResponse
        | VectorStoreRecordKind::IndexEndpoints
        | VectorStoreRecordKind::BedrockRequest
        | VectorStoreRecordKind::BedrockRow => return Ok(()),
    };
    let Some(results) = record.get(field).and_then(Value::as_array) else {
        return Ok(());
    };
    for result in results {
        if let Some(score) = result.get("score") {
            validate_score(score)?;
        }
    }
    Ok(())
}

fn validate_score(value: &Value) -> Result<(), VectorStoreRecordError> {
    let Some(number) = value.as_number() else {
        return Ok(());
    };
    let text = number.to_string();
    let decimal = Decimal::from_str_exact(&text).or_else(|_| Decimal::from_scientific(&text));
    if decimal.is_err() {
        return Err(VectorStoreRecordError::ScoreOutOfRange);
    }
    Ok(())
}

#[serde_with::serde_as]
#[derive(Deserialize)]
#[serde(transparent)]
struct Members<'input>(
    #[serde_as(as = "serde_with::Map<_, _>")]
    #[serde(borrow)]
    Vec<(IgnoredAny, &'input RawValue)>,
);
