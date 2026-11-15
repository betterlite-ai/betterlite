use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::safe_json_loads;
use betterlite_redact::structured::SensitiveDataMasker;
use chrono::{DateTime, FixedOffset};
use secrecy::SecretString;

use super::enums::{AuditAction, AuditName, AuditTable};

const REQUIRED_FIELDS: [&str; 5] = ["id", "updated_at", "action", "table_name", "object_id"];
const OPTIONAL_FIELDS: [&str; 4] = [
    "changed_by",
    "changed_by_api_key",
    "before_value",
    "updated_values",
];

#[derive(Debug, thiserror::Error)]
pub enum AuditRecordError {
    #[error("the audit record is not valid JSON: {0}")]
    Json(#[from] PrivateJsonError),
    #[error("the audit record is not a JSON object")]
    NotAnObject,
    #[error("the audit record field {field} is required")]
    MissingField { field: &'static str },
    #[error("the audit record field {field} must be a string")]
    FieldType { field: &'static str },
    #[error("the audit record field {field} is not an allowed value")]
    UnknownValue { field: &'static str },
    #[error("the audit record field {field} is not an ISO-8601 timestamp")]
    Timestamp {
        field: &'static str,
        #[source]
        source: chrono::ParseError,
    },
    #[error("the audit record field {field} is not valid JSON: {source}")]
    JsonField {
        field: &'static str,
        #[source]
        source: PrivateJsonError,
    },
    #[error("the audit record has an unknown field {field}")]
    UnknownField { field: String },
}

#[derive(Clone, Debug)]
pub struct AuditLogRecord {
    pub id: String,
    pub updated_at: DateTime<FixedOffset>,
    pub changed_by: Option<String>,
    pub changed_by_api_key: Option<SecretString>,
    pub action: AuditAction,
    pub table_name: AuditTable,
    pub object_id: String,
    pub before_value: Option<JsonValue>,
    pub updated_values: Option<JsonValue>,
}

impl AuditLogRecord {
    pub fn masked(mut record: Self) -> Result<Self, AuditRecordError> {
        record.before_value = masked_audit_value(record.before_value, "before_value")?;
        record.updated_values = masked_audit_value(record.updated_values, "updated_values")?;
        Ok(record)
    }
}

pub fn decode_audit_record(bytes: &[u8]) -> Result<AuditLogRecord, AuditRecordError> {
    match safe_json_loads(bytes) {
        Ok(value) => record_from_json(&value),
        Err(source) => Err(AuditRecordError::from(source)),
    }
}

fn record_from_json(value: &JsonValue) -> Result<AuditLogRecord, AuditRecordError> {
    if !matches!(value, JsonValue::Object(_)) {
        return Err(AuditRecordError::NotAnObject);
    }
    reject_unknown_fields(value)?;
    let id = required_string(value, "id")?;
    let updated_at = parse_timestamp(&required_string(value, "updated_at")?)?;
    let action = AuditAction::parse(&required_string(value, "action")?)
        .ok_or(AuditRecordError::UnknownValue { field: "action" })?;
    let table_name = AuditTable::parse(&required_string(value, "table_name")?).ok_or(
        AuditRecordError::UnknownValue {
            field: "table_name",
        },
    )?;
    let object_id = required_string(value, "object_id")?;
    AuditLogRecord::masked(AuditLogRecord {
        id,
        updated_at,
        changed_by: changed_by_text(value.get("changed_by")),
        changed_by_api_key: optional_string(value, "changed_by_api_key")?.map(SecretString::from),
        action,
        table_name,
        object_id,
        before_value: value.get("before_value").cloned(),
        updated_values: value.get("updated_values").cloned(),
    })
}

fn reject_unknown_fields(value: &JsonValue) -> Result<(), AuditRecordError> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    object
        .keys()
        .find(|key| {
            !REQUIRED_FIELDS.contains(&key.as_str()) && !OPTIONAL_FIELDS.contains(&key.as_str())
        })
        .map_or(Ok(()), |field| {
            Err(AuditRecordError::UnknownField {
                field: field.clone(),
            })
        })
}

fn required_string(value: &JsonValue, field: &'static str) -> Result<String, AuditRecordError> {
    match value.get(field) {
        None => Err(AuditRecordError::MissingField { field }),
        Some(JsonValue::String(text)) => Ok(text.clone()),
        Some(_) => Err(AuditRecordError::FieldType { field }),
    }
}

fn optional_string(
    value: &JsonValue,
    field: &'static str,
) -> Result<Option<String>, AuditRecordError> {
    match value.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(AuditRecordError::FieldType { field }),
    }
}

fn changed_by_text(value: Option<&JsonValue>) -> Option<String> {
    match value? {
        JsonValue::Null => None,
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Bool(flag) => Some(if *flag { "True" } else { "False" }.to_owned()),
        number @ JsonValue::Number(_) => Some(number.to_string()),
        complex @ (JsonValue::Array(_) | JsonValue::Object(_)) => Some(complex.to_string()),
    }
}

fn parse_timestamp(text: &str) -> Result<DateTime<FixedOffset>, AuditRecordError> {
    DateTime::parse_from_rfc3339(text).map_err(|source| AuditRecordError::Timestamp {
        field: "updated_at",
        source,
    })
}

fn masked_audit_value(
    value: Option<JsonValue>,
    field: &'static str,
) -> Result<Option<JsonValue>, AuditRecordError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = match value {
        JsonValue::Null => return Ok(None),
        JsonValue::String(text) => safe_json_loads(text.as_bytes())
            .map_err(|source| AuditRecordError::JsonField { field, source })?,
        value @ (JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_)) => value,
    };
    Ok(Some(
        SensitiveDataMasker::with_patterns(["key"], ["cost"]).mask(&value),
    ))
}

#[must_use]
pub fn payload_text(value: &JsonValue) -> String {
    match value {
        JsonValue::String(text) => text.clone(),
        stored => stored.to_string(),
    }
}

// A UTC instant reads Z in stored JSON and +00:00 in Python's isoformat, so the caller names its form
#[must_use]
pub fn timestamp_text(value: DateTime<FixedOffset>, utc_suffix: &str) -> String {
    let seconds = value.format("%Y-%m-%dT%H:%M:%S");
    let micros = value.timestamp_subsec_micros();
    let zone = if value.offset().local_minus_utc() == 0 {
        utc_suffix.to_owned()
    } else {
        value.format("%:z").to_string()
    };
    if micros == 0 {
        format!("{seconds}{zone}")
    } else {
        format!("{seconds}.{micros:06}{zone}")
    }
}
