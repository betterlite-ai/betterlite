use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::resource_rows::model::ProxyModelTable;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::request_pricing::fields::echoed_cost_map_pricing_fields;
use chrono::NaiveDateTime;

use super::convert::json_of;
use super::warnings::Warnings;
use crate::audit_events::record::timestamp_text;

const ID: &str = "id";
const DB_MODEL: &str = "db_model";
const BLOCKED: &str = "blocked";
const MEMBER_AUTO_ROUTER: &str = "member_auto_router";
const CREATED_AT: &str = "created_at";
const UPDATED_AT: &str = "updated_at";
const CREATED_BY: &str = "created_by";
const UPDATED_BY: &str = "updated_by";

// get_model_info_with_id: the row's id, db_model and blocked flags, the audit columns, and no echoed cost map.
#[must_use]
pub fn model_info_for(row: &ProxyModelTable, db_model: bool, warnings: &Warnings) -> JsonObject {
    let fields = &row.fields;
    let Some(payload) = &fields.model_info else {
        // LiteLLM indexes a missing dict and raises; the minimal RouterModelInfo keeps the row's id usable.
        let mut info = JsonObject::new();
        info.insert(ID.to_owned(), Json::Text(fields.model_id.clone()));
        info.insert(DB_MODEL.to_owned(), Json::Bool(db_model));
        info.insert(MEMBER_AUTO_ROUTER.to_owned(), Json::Bool(false));
        return info;
    };
    let mut info: JsonObject = payload
        .0
        .iter()
        .map(|(name, value)| (name.clone(), json_of(value)))
        .collect();
    // the row's model_id always exists, so LiteLLM's id fallback and db_model-flag fallback never run for a database row
    info.insert(ID.to_owned(), Json::Text(fields.model_id.clone()));
    info.insert(DB_MODEL.to_owned(), Json::Bool(true));
    info.insert(BLOCKED.to_owned(), Json::Bool(fields.blocked));
    insert_audit_fields(&mut info, row);
    drop_echoed_pricing(&mut info, &fields.model_id, warnings);
    finish_router_info(info)
}

// premium_user is always true here, so each audit column overwrites whatever model_info stored, null included
fn insert_audit_fields(info: &mut JsonObject, row: &ProxyModelTable) {
    let fields = &row.fields;
    let created_at = fields
        .created_at
        .map_or(Json::Null, |time| Json::Text(json_timestamp(time)));
    let updated_at = fields
        .updated_at
        .map_or(Json::Null, |time| Json::Text(json_timestamp(time)));
    let created_by = fields
        .created_by
        .as_ref()
        .map_or(Json::Null, |name| Json::Text(name.clone()));
    let updated_by = fields
        .updated_by
        .as_ref()
        .map_or(Json::Null, |name| Json::Text(name.clone()));
    for (name, value) in [
        (CREATED_AT, created_at),
        (UPDATED_AT, updated_at),
        (CREATED_BY, created_by),
        (UPDATED_BY, updated_by),
    ] {
        info.insert(name.to_owned(), value);
    }
}

fn drop_echoed_pricing(info: &mut JsonObject, model_id: &str, warnings: &Warnings) {
    let echoed = echoed_cost_map_pricing_fields(info);
    if echoed.is_empty() {
        return;
    }
    if warnings.first_cost_map_copy(model_id, &echoed) {
        tracing::warn!(
            "Deployment {model_id} stores a copy of the cost map in model_info ({}); ignoring it so the deployment follows the current cost map. Set the price on litellm_params to override the cost map on purpose.",
            echoed.join(", "),
        );
    }
    info.retain(|name, _value| !echoed.contains(name));
}

// RouterModelInfo(**fields).model_dump(exclude_none=True): no null survives and the false default is written out.
fn finish_router_info(mut info: JsonObject) -> JsonObject {
    info.retain(|_name, value| !matches!(value, Json::Null));
    if !matches!(info.get(MEMBER_AUTO_ROUTER), Some(Json::Bool(_))) {
        info.insert(MEMBER_AUTO_ROUTER.to_owned(), Json::Bool(false));
    }
    info
}

// pydantic's mode="json" datetime: Z for UTC, the numeric offset otherwise, micros only when present.
fn json_timestamp(value: Timestamp) -> String {
    match value {
        Timestamp::Aware(time) => timestamp_text(time, "Z"),
        Timestamp::Naive(time) => naive_text(time),
    }
}

fn naive_text(value: NaiveDateTime) -> String {
    if value.and_utc().timestamp_subsec_micros() == 0 {
        value.format("%Y-%m-%dT%H:%M:%S").to_string()
    } else {
        value.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()
    }
}
