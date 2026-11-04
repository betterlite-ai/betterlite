use betterlite_models::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
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
    #[error("Field required")]
    MissingField(String),
    #[error("invalid literal for field `{field}`: {value}")]
    InvalidLiteral { field: String, value: String },
    #[error("score cannot be represented as an exact decimal")]
    ScoreOutOfRange(#[source] DecimalLexemeError),
}

#[derive(Debug, PartialEq, Eq)]
pub struct ErrorDetails<'error> {
    pub status: Option<u16>,
    pub error_type: Option<&'static str>,
    pub param: Option<&'static str>,
    pub code: Option<&'static str>,
    pub location: Option<&'error str>,
}

impl VectorStoreRecordError {
    #[must_use]
    pub const fn details(&self) -> ErrorDetails<'_> {
        let (status, error_type, param, code, location) = match self {
            Self::ScoreOutOfRange(_) => (
                Some(502),
                Some("bad_gateway"),
                Some("score"),
                Some("decimal_out_of_range"),
                None,
            ),
            Self::MissingField(field) => (None, Some("missing"), None, None, Some(field.as_str())),
            Self::InvalidLiteral { field, .. } => (
                None,
                Some("literal_error"),
                None,
                None,
                Some(field.as_str()),
            ),
            Self::UnknownKind(_)
            | Self::JsonTooLarge
            | Self::TooDeep
            | Self::TooManyItems
            | Self::Json(_) => (None, None, None, None, None),
        };
        ErrorDetails {
            status,
            error_type,
            param,
            code,
            location,
        }
    }
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
    validate_record_shape(kind, &parsed)?;
    if kind == VectorStoreRecordKind::SearchResponse {
        validate_result_scores(parsed.get("data"))?;
    }
    if kind == VectorStoreRecordKind::BedrockResponse {
        validate_result_scores(parsed.get("retrievalResults"))?;
    }
    VectorStoreRecord::from_value(kind, parsed)
}

fn validate_record_shape(
    kind: VectorStoreRecordKind,
    parsed: &Value,
) -> Result<(), VectorStoreRecordError> {
    if matches!(
        kind,
        VectorStoreRecordKind::UpdateRequest
            | VectorStoreRecordKind::DeleteRequest
            | VectorStoreRecordKind::InfoRequest
    ) && parsed.get("vector_store_id").is_none()
    {
        return Err(VectorStoreRecordError::MissingField(String::from(
            "vector_store_id",
        )));
    }
    if let Some(object) = parsed
        .get("object")
        .filter(|object| kind == VectorStoreRecordKind::IndexListResponse && *object != "list")
    {
        return Err(VectorStoreRecordError::InvalidLiteral {
            field: String::from("object"),
            value: object.to_string(),
        });
    }
    Ok(())
}

fn validate_result_scores(value: Option<&Value>) -> Result<(), VectorStoreRecordError> {
    let Some(results) = value.and_then(Value::as_array) else {
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
    parse_decimal_lexeme(&text).map_err(VectorStoreRecordError::ScoreOutOfRange)?;
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
