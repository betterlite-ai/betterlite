use betterlite_core::api::responses_stream_primitives::error_fields::{
    StreamErrorAttributes, StreamErrorObject, error_event_fields, stream_error_status_and_message,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// an in-stream error arrives either as the decoded event body or as a provider error object
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorKind {
    Mapping,
    Attributes,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ErrorRow {
    kind: ErrorKind,
    #[serde(default)]
    fields: IndexMap<String, Json>,
}

pub(crate) fn answer(rows: &[ErrorRow]) -> Value {
    let answers: Vec<Value> = rows.iter().map(row_answer).collect();
    json!({ "rows": answers })
}

fn row_answer(row: &ErrorRow) -> Value {
    let fields = error_event_fields(object_of(row));
    let (status, message) = stream_error_status_and_message(object_of(row));
    json!({
        "kind": row.kind,
        "message": message,
        "type": fields.error_type,
        "code": fields.code,
        "status": status,
    })
}

fn object_of(row: &ErrorRow) -> StreamErrorObject<'_> {
    match row.kind {
        ErrorKind::Mapping => StreamErrorObject::Mapping(&row.fields),
        ErrorKind::Attributes => StreamErrorObject::Attributes(StreamErrorAttributes {
            message: row.fields.get("message"),
            error_type: row.fields.get("type"),
            code: row.fields.get("code"),
        }),
    }
}
