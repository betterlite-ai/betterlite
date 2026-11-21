use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::api_boundaries::ApiLimits;
use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::error::RequestMetadataSafetyError;

const LITELLM_METADATA_ROUTES: [&str; 5] =
    ["batches", "bedrock", "/v1/messages", "responses", "files"];
const TRACE_CONTROL_FIELDS: [&str; 9] = [
    "mask_input",
    "mask_output",
    "session_id",
    "trace_id",
    "trace_metadata",
    "trace_name",
    "trace_release",
    "trace_user_id",
    "trace_version",
];

// the two request-body slots LiteLLM keeps metadata in; the route decides which one a request uses
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataField {
    Metadata,
    LitellmMetadata,
}

impl MetadataField {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::LitellmMetadata => "litellm_metadata",
        }
    }
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn metadata_variable_name_for_route(route: &str) -> MetadataField {
    if route.contains("thread") || route.contains("assistant") {
        return MetadataField::LitellmMetadata;
    }
    if LITELLM_METADATA_ROUTES
        .iter()
        .any(|metadata_route| route.contains(metadata_route))
    {
        return MetadataField::LitellmMetadata;
    }
    MetadataField::Metadata
}

#[must_use]
pub fn metadata_name_from_data(data: &RequestData) -> MetadataField {
    if data.contains_key(MetadataField::LitellmMetadata.as_str()) {
        MetadataField::LitellmMetadata
    } else {
        MetadataField::Metadata
    }
}

// the JSON type names a metadata error reports, which keeps the 400 actionable without echoing the value
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonType {
    Null,
    Boolean,
    Integer,
    Number,
    Text,
    List,
    Object,
}

impl JsonType {
    pub(crate) const fn describe(self) -> &'static str {
        match self {
            Self::Null => "a NoneType",
            Self::Boolean => "a boolean",
            Self::Integer => "an integer",
            Self::Number => "a number",
            Self::Text => "a string",
            Self::List => "an array",
            Self::Object => "a dict",
        }
    }
}

impl From<&Json> for JsonType {
    fn from(value: &Json) -> Self {
        match value {
            Json::Null => Self::Null,
            Json::Bool(_) => Self::Boolean,
            // Python's json parser reads a literal with no fraction and no exponent as an int
            Json::Number(number) => {
                if number.to_string().contains(['.', 'e', 'E']) {
                    Self::Number
                } else {
                    Self::Integer
                }
            }
            Json::Text(_) => Self::Text,
            Json::List(_) => Self::List,
            Json::Object(_) => Self::Object,
        }
    }
}

// a metadata object arrives as an object or as JSON text in an object, and anything else is a 400
pub fn normalized_metadata_object(
    field: MetadataField,
    value: &Json,
    limits: ApiLimits,
) -> Result<IndexMap<String, Json>, RequestMetadataSafetyError> {
    let normalized = match value {
        Json::Object(metadata) => return Ok(metadata.clone()),
        Json::Text(text) => parsed_object(text, limits),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => None,
    };
    normalized.ok_or_else(|| RequestMetadataSafetyError::MetadataNotObject {
        field,
        received: JsonType::from(value),
    })
}

// the request's own slot is replaced by an object in place, and a value that is not one leaves an empty object
pub fn normalized_metadata_slot(data: &mut RequestData, field: MetadataField, limits: ApiLimits) {
    let name = field.as_str();
    if matches!(data.get(name), Some(Json::Object(_))) {
        return;
    }
    let replacement = match data.get(name) {
        Some(Json::Text(text)) => parsed_object(text, limits).unwrap_or_default(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => IndexMap::new(),
    };
    data.insert(name.to_owned(), Json::Object(replacement));
}

// the request body's own cap already bounds every metadata string, and checking it here bounds a helper call too
fn parsed_object(text: &str, limits: ApiLimits) -> Option<IndexMap<String, Json>> {
    if text.len() > limits.max_json_bytes {
        return None;
    }
    match serde_json::from_str::<Json>(text) {
        Ok(Json::Object(metadata)) => Some(metadata),
        Ok(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | Err(_) => None,
    }
}

// the caller's trace-control fields that the litellm_metadata slot does not already set
#[must_use]
pub fn promoted_trace_control_fields(
    requester_metadata: &IndexMap<String, Json>,
    litellm_metadata: &IndexMap<String, Json>,
) -> Vec<(String, Json)> {
    let mut promoted = Vec::new();
    for (key, value) in requester_metadata {
        if TRACE_CONTROL_FIELDS.contains(&key.as_str()) && !litellm_metadata.contains_key(key) {
            promoted.push((key.clone(), value.clone()));
        }
    }
    promoted
}
