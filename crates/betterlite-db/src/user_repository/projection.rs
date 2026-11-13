use crate::budget_permission_rows::JsonData;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::postgres_store::diagnostics::DriverError;
use crate::resource_rows::{ResourceRowsError, parse_data};

pub const MAX_USER_FIELD_BYTES: usize = 0x0010_0000;
pub const MAX_USER_JSON_DEPTH: usize = 64;
// LiteLLM decodes exactly these stored columns from JSON text; every other column keeps its stored value
const JSON_ENCODED_FIELDS: [&str; 3] = ["metadata", "model_spend", "model_max_budget"];

#[derive(Debug, thiserror::Error)]
pub enum UserRecordError {
    #[error("the user row column is unreadable")]
    Column {
        #[source]
        source: DriverError,
    },
    #[error("the stored user record is not a readable row")]
    Record {
        #[source]
        source: ResourceRowsError,
    },
    #[error("the stored user record cannot be converted")]
    Convert {
        #[source]
        source: serde_json::Error,
    },
    #[error("the stored user record needs to be a JSON object")]
    NotAnObject,
    #[error(
        "the stored user field {field} is {len} bytes, over the {MAX_USER_FIELD_BYTES} byte limit"
    )]
    FieldTooLarge { field: &'static str, len: usize },
    #[error("the stored user field {field} exceeds JSON depth {MAX_USER_JSON_DEPTH}")]
    FieldTooDeep { field: &'static str },
    #[error("the stored user field {field} is not valid JSON")]
    FieldJson {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the stored user row does not match the user model")]
    Model {
        #[source]
        source: serde_json::Error,
    },
}

pub fn decode_user_record(bytes: &[u8]) -> Result<Option<LiteLLMUserTable>, UserRecordError> {
    match parse_data(bytes) {
        Ok(record) => user_record_to_model(Some(&record)),
        Err(source) => Err(UserRecordError::Record { source }),
    }
}

pub fn user_record_to_model(
    record: Option<&JsonData>,
) -> Result<Option<LiteLLMUserTable>, UserRecordError> {
    let fields = match record {
        None | Some(JsonData::Null) => return Ok(None),
        Some(JsonData::Object(fields)) => fields,
        Some(_) => return Err(UserRecordError::NotAnObject),
    };
    let mut decoded = fields.clone();
    for field in JSON_ENCODED_FIELDS {
        if let Some(JsonData::String(text)) = decoded.get(field).cloned() {
            decoded.insert(field.to_owned(), decode_json_field(field, &text)?);
        }
    }
    let text = serde_json::to_string(&JsonData::Object(decoded))
        .map_err(|source| UserRecordError::Convert { source })?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| UserRecordError::Model { source })
}

fn decode_json_field(field: &'static str, text: &str) -> Result<JsonData, UserRecordError> {
    if text.len() > MAX_USER_FIELD_BYTES {
        return Err(UserRecordError::FieldTooLarge {
            field,
            len: text.len(),
        });
    }
    if exceeds_depth(text, MAX_USER_JSON_DEPTH) {
        return Err(UserRecordError::FieldTooDeep { field });
    }
    serde_json::from_str(text).map_err(|source| UserRecordError::FieldJson { field, source })
}

fn exceeds_depth(text: &str, limit: usize) -> bool {
    let mut depth = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for byte in text.bytes() {
        match (quoted, escaped, byte) {
            (true, true, _) => escaped = false,
            (true, false, b'\\') => escaped = true,
            (true, false, b'"') => quoted = false,
            (false, _, b'"') => quoted = true,
            (false, _, b'{' | b'[') => {
                depth = depth.saturating_add(1);
                if depth > limit {
                    return true;
                }
            }
            (false, _, b'}' | b']') => depth = depth.saturating_sub(1),
            (true, false, _) | (false, _, _) => {}
        }
    }
    false
}
