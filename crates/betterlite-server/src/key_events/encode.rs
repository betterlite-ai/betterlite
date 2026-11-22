use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::{Expiry, LiteLLMVerificationToken};
use betterlite_openai::openai_base_config::Object as JsonMap;
use chrono::{DateTime, FixedOffset, NaiveDateTime};

use crate::audit_events::record::timestamp_text;
use crate::key_fields::error::DecodeError;
use crate::key_fields::response::GenerateKeyResponse;
use crate::key_fields::update::UpdateKeyRequest;

const RESPONSE_INSTANTS: [&str; 3] = ["expires", "created_at", "updated_at"];
const UPDATE_INSTANTS: [&str; 1] = ["temp_budget_expiry"];
const ROW_INSTANTS: [&str; 7] = [
    "budget_reset_at",
    "created_at",
    "updated_at",
    "settings_updated_at",
    "last_active",
    "last_rotation_at",
    "key_rotation_at",
];

// Pydantic has two datetime renderings the stored strings must copy: model_dump_json writes an aware UTC instant as Z, while json.dumps(default=str) writes str(datetime) with a space and +00:00
#[derive(Clone, Copy)]
enum InstantForm {
    Json,
    Python,
}

enum Instant {
    Aware(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
}

// response.model_dump_json(exclude_none=True)
pub fn response_dump(response: &GenerateKeyResponse) -> Result<String, DecodeError> {
    let mut value = object_of(&response.project()?.resolved)?;
    exclude_none(&mut value);
    rewrite_instants(&mut value, &RESPONSE_INSTANTS, InstantForm::Json);
    rewrite_windows(&mut value, InstantForm::Json);
    serde_json::to_string(&value).map_err(DecodeError::Document)
}

// json.dumps(data.model_dump(exclude_none=True), default=str); an explicit null project_id is restored after masking
pub fn update_dump(data: &UpdateKeyRequest) -> Result<String, DecodeError> {
    let mut value = object_of(&data.project()?.resolved)?;
    exclude_none(&mut value);
    rewrite_instants(&mut value, &UPDATE_INSTANTS, InstantForm::Python);
    rewrite_windows(&mut value, InstantForm::Python);
    serde_json::to_string(&value).map_err(DecodeError::Document)
}

// row.model_dump_json(exclude_none=True)
pub fn row_dump(row: &LiteLLMVerificationToken) -> Result<String, serde_json::Error> {
    let mut value = serde_json::to_value(row)?;
    rewrite_expiry(&mut value, row.expires.as_ref(), InstantForm::Json);
    rewrite_row(&mut value, InstantForm::Json);
    serde_json::to_string(&value)
}

// json.dumps(existing_key_row.json(exclude_none=True), default=str)
pub fn row_legacy_dump(row: &LiteLLMVerificationToken) -> Result<String, serde_json::Error> {
    let mut value = serde_json::to_value(row)?;
    rewrite_expiry(&mut value, row.expires.as_ref(), InstantForm::Python);
    rewrite_row(&mut value, InstantForm::Python);
    serde_json::to_string(&value)
}

fn rewrite_row(value: &mut JsonValue, form: InstantForm) {
    rewrite_instants(value, &ROW_INSTANTS, form);
    exclude_row_none(value);
}

fn object_of(values: &BTreeMap<String, JsonData>) -> Result<JsonValue, DecodeError> {
    let value = serde_json::to_value(values).map_err(DecodeError::Document)?;
    match value {
        JsonValue::Object(fields) => Ok(JsonValue::Object(fields)),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => Err(DecodeError::NotAnObject),
    }
}

// pydantic's exclude_none also drops nulls inside a nested model, never inside a plain dict; the router settings model then keeps its own non-null default for model_group_alias
fn exclude_none(value: &mut JsonValue) {
    let JsonValue::Object(fields) = value else {
        return;
    };
    fields.retain(|_, field| !field.is_null());
    if let Some(permission) = fields.get_mut("object_permission") {
        exclude_none_field(permission);
    }
    if let Some(JsonValue::Object(router)) = fields.get_mut("router_settings") {
        router.retain(|_, field| !field.is_null());
        router
            .entry("model_group_alias".to_owned())
            .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    }
    if let Some(JsonValue::Array(entries)) = fields.get_mut("budget_limits") {
        for entry in entries {
            exclude_none_field(entry);
        }
    }
}

fn exclude_none_field(value: &mut JsonValue) {
    let JsonValue::Object(fields) = value else {
        return;
    };
    fields.retain(|_, field| !field.is_null());
}

fn exclude_row_none(value: &mut JsonValue) {
    let JsonValue::Object(fields) = value else {
        return;
    };
    fields.retain(|_, field| !field.is_null());
    if let Some(permission) = fields.get_mut("object_permission") {
        exclude_none_field(permission);
    }
}

fn rewrite_instants(value: &mut JsonValue, names: &[&str], form: InstantForm) {
    let JsonValue::Object(fields) = value else {
        return;
    };
    for name in names {
        if let Some(field) = fields.get_mut(*name) {
            rewrite_instant(field, form);
        }
    }
}

fn rewrite_windows(value: &mut JsonValue, form: InstantForm) {
    let Some(JsonValue::Array(entries)) = value.pointer_mut("/budget_limits") else {
        return;
    };
    for entry in entries {
        rewrite_window(entry, form);
    }
}

fn rewrite_window(entry: &mut JsonValue, form: InstantForm) {
    let JsonValue::Object(fields) = entry else {
        return;
    };
    if let Some(window) = fields.get_mut("reset_at") {
        rewrite_instant(window, form);
    }
}

fn rewrite_instant(value: &mut JsonValue, form: InstantForm) {
    let JsonValue::String(text) = value else {
        return;
    };
    if let Some(instant) = parse_instant(text) {
        *text = render_instant(&instant, form);
    }
}

// a stored expiry typed str keeps its text; only a datetime value is re-rendered
fn rewrite_expiry(value: &mut JsonValue, expires: Option<&Expiry>, form: InstantForm) {
    let Some(Expiry::DateTime(instant)) = expires else {
        return;
    };
    let JsonValue::Object(fields) = value else {
        return;
    };
    fields.insert(
        "expires".to_owned(),
        JsonValue::String(render_instant(&Instant::Aware(*instant), form)),
    );
}

fn parse_instant(text: &str) -> Option<Instant> {
    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Some(Instant::Aware(value));
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .map(Instant::Naive)
}

fn render_instant(instant: &Instant, form: InstantForm) -> String {
    match instant {
        Instant::Aware(value) => match form {
            InstantForm::Json => timestamp_text(*value, "Z"),
            InstantForm::Python => aware_legacy_text(*value),
        },
        Instant::Naive(value) => naive_text(*value, form),
    }
}

fn aware_legacy_text(value: DateTime<FixedOffset>) -> String {
    let seconds = value.format("%Y-%m-%d %H:%M:%S");
    let micros = value.timestamp_subsec_micros();
    let zone = value.format("%:z");
    if micros == 0 {
        format!("{seconds}{zone}")
    } else {
        format!("{seconds}.{micros:06}{zone}")
    }
}

fn naive_text(value: NaiveDateTime, form: InstantForm) -> String {
    let micros = value.and_utc().timestamp_subsec_micros();
    let seconds = match form {
        InstantForm::Json => value.format("%Y-%m-%dT%H:%M:%S"),
        InstantForm::Python => value.format("%Y-%m-%d %H:%M:%S"),
    };
    if micros == 0 {
        return seconds.to_string();
    }
    format!("{seconds}.{micros:06}")
}
