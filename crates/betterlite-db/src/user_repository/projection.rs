use crate::budget_permission_rows::JsonData;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::resource_rows::{ResourceRowsError, parse_data};

// LiteLLM decodes exactly these stored columns from JSON text; every other column keeps its stored value
const JSON_ENCODED_FIELDS: [&str; 3] = ["metadata", "model_spend", "model_max_budget"];

#[derive(Debug, thiserror::Error)]
pub enum UserRecordError {
    #[error("the stored user record is not a readable row")]
    Record {
        #[source]
        source: ResourceRowsError,
    },
    #[error("the stored user field {field} is not a readable record")]
    Field {
        field: &'static str,
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
            let value = parse_data(text.as_bytes())
                .map_err(|source| UserRecordError::Field { field, source })?;
            decoded.insert(field.to_owned(), value);
        }
    }
    let text = serde_json::to_string(&JsonData::Object(decoded))
        .map_err(|source| UserRecordError::Convert { source })?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| UserRecordError::Model { source })
}
