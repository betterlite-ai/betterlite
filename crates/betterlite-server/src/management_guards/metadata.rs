use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use betterlite_db::budget_permission_rows::JsonData;

use crate::management_guards::error::ManagementGuardError;

// LiteLLM's literal order decides which field moves first and which error comes first
pub const PREMIUM_METADATA_FIELDS: [&str; 9] = [
    "disable_global_guardrails",
    "guardrails",
    "policies",
    "tags",
    "team_member_key_duration",
    "prompts",
    "logging",
    "secret_manager_settings",
    "allowed_passthrough_routes",
];

pub const METADATA_FIELDS: [&str; 19] = [
    "model_rpm_limit",
    "model_tpm_limit",
    "model_itpm_limit",
    "model_otpm_limit",
    "default_estimated_output_tokens",
    "default_estimated_output_tokens_per_model",
    "mcp_rpm_limit",
    "tag_rpm_limit",
    "rpm_limit_type",
    "tpm_limit_type",
    "enforced_params",
    "temp_budget_increase",
    "temp_budget_expiry",
    "allowed_vector_store_indexes",
    "enforced_batch_output_expires_after",
    "enforced_file_expires_after",
    "throttle_on_budget_exceeded",
    "enable_prompt_caching",
    "end_user_budget_id",
];

#[must_use]
pub fn has_effect(value: &JsonData) -> bool {
    match value {
        JsonData::Null => false,
        JsonData::Bool(flag) => *flag,
        JsonData::Number(number) => number
            .as_f64()
            .is_some_and(|magnitude| magnitude != 0.0_f64),
        JsonData::String(text) => !text.is_empty(),
        JsonData::Array(items) => !items.is_empty(),
        JsonData::Object(fields) => !fields.is_empty(),
    }
}

pub fn set_object_metadata_field(
    metadata: &mut Option<BTreeMap<String, JsonData>>,
    field_name: &str,
    value: JsonData,
) {
    metadata
        .get_or_insert_with(BTreeMap::new)
        .insert(field_name.to_owned(), value);
}

pub fn update_metadata_field(
    updated_kv: &mut BTreeMap<String, JsonData>,
    field_name: &str,
) -> Result<(), ManagementGuardError> {
    let Some(value) = updated_kv.remove(field_name) else {
        return Ok(());
    };
    if matches!(value, JsonData::Null) {
        updated_kv.insert(field_name.to_owned(), value);
        return Ok(());
    }
    // the pop above stands even when the move fails, exactly like the source dict
    match updated_kv.entry("metadata".to_owned()) {
        Entry::Occupied(mut occupied) => match occupied.get_mut() {
            JsonData::Object(fields) => {
                fields.insert(field_name.to_owned(), value);
            }
            metadata @ JsonData::Null => {
                *metadata = JsonData::Object(BTreeMap::from([(field_name.to_owned(), value)]));
            }
            JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Array(_) => {
                return Err(ManagementGuardError::MetadataNotAnObject {
                    field: field_name.to_owned(),
                });
            }
        },
        Entry::Vacant(vacant) => {
            vacant.insert(JsonData::Object(BTreeMap::from([(
                field_name.to_owned(),
                value,
            )])));
        }
    }
    Ok(())
}

pub fn update_metadata_fields(
    updated_kv: &mut BTreeMap<String, JsonData>,
) -> Result<(), ManagementGuardError> {
    for field in PREMIUM_METADATA_FIELDS.into_iter().chain(METADATA_FIELDS) {
        let movable = updated_kv
            .get(field)
            .is_some_and(|value| !matches!(value, JsonData::Null));
        if movable {
            update_metadata_field(updated_kv, field)?;
        }
    }
    Ok(())
}
