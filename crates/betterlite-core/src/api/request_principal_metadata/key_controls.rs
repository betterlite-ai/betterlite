use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::request_destination::RequestData;
use crate::api::request_metadata_safety::metadata::MetadataField;
use crate::api::request_principal_metadata::principal::CALLBACK_CONFIG_SLOTS;
use crate::api::request_principal_metadata::tags::{TAGS_KEY, merge_tags};
use crate::api::request_principal_metadata::{Metadata, take_bucket};

const CACHE_KEY: &str = "cache";
const SUPPORTED_CACHE_CONTROLS: [&str; 4] = ["ttl", "s-maxage", "no-cache", "no-store"];
const SPEND_LOGS_KEY: &str = "spend_logs_metadata";
const GUARDRAILS_FLAG: &str = "disable_global_guardrails";
// a key's own boolean for these goes on the request body itself, outside the metadata slot
const BODY_FLAGS: [&str; 2] = ["disable_fallbacks", "enable_prompt_caching"];
const AUTH_METADATA_KEY: &str = "user_api_key_auth_metadata";
// LiteLLM_ManagementEndpoint_MetadataFields_Premium then LiteLLM_ManagementEndpoint_MetadataFields: proxy controls, not caller facts
const MANAGEMENT_FIELDS: [&str; 28] = [
    "disable_global_guardrails",
    "guardrails",
    "policies",
    "tags",
    "team_member_key_duration",
    "prompts",
    "logging",
    "secret_manager_settings",
    "allowed_passthrough_routes",
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

pub fn add_key_level_controls(
    data: &mut RequestData,
    slot: MetadataField,
    key_metadata: &Metadata,
) {
    // a cache entry that is not an object still resets the request's cache controls to none
    if let Some(cache) = key_metadata.get(CACHE_KEY) {
        let controls = if let Json::Object(requested) = cache {
            requested
                .iter()
                .filter(|(control, _value)| SUPPORTED_CACHE_CONTROLS.contains(&control.as_str()))
                .map(|(control, value)| (control.clone(), value.clone()))
                .collect()
        } else {
            IndexMap::new()
        };
        data.insert(CACHE_KEY.to_owned(), Json::Object(controls));
    }
    let mut bucket = take_bucket(data, slot);
    if let Some(key_tags) = key_metadata
        .get(TAGS_KEY)
        .filter(|tags| !matches!(tags, Json::Null))
    {
        let added: &[Json] = if let Json::List(tags) = key_tags {
            tags
        } else {
            &[]
        };
        let merged = merge_tags(bucket.get(TAGS_KEY), added);
        bucket.insert(TAGS_KEY.to_owned(), Json::List(merged));
    }
    if let Some(flag @ Json::Bool(_)) = key_metadata.get(GUARDRAILS_FLAG) {
        bucket.insert(GUARDRAILS_FLAG.to_owned(), flag.clone());
    }
    if let Some(Json::Object(key_spend_logs)) = key_metadata.get(SPEND_LOGS_KEY) {
        merge_spend_logs_metadata(&mut bucket, key_spend_logs);
    }
    data.insert(slot.as_str().to_owned(), Json::Object(bucket));
    for name in BODY_FLAGS {
        if let Some(flag @ Json::Bool(_)) = key_metadata.get(name) {
            data.insert(name.to_owned(), flag.clone());
        }
    }
    add_management_metadata(data, slot, key_metadata);
}

// the caller's own spend-log entries win, and the key only fills the names the caller left out
fn merge_spend_logs_metadata(bucket: &mut Metadata, key_spend_logs: &Metadata) {
    if let Some(Json::Object(request_spend_logs)) = bucket.get_mut(SPEND_LOGS_KEY) {
        for (name, value) in key_spend_logs {
            request_spend_logs
                .entry(name.clone())
                .or_insert_with(|| value.clone());
        }
    } else {
        bucket.insert(
            SPEND_LOGS_KEY.to_owned(),
            Json::Object(key_spend_logs.clone()),
        );
    }
}

// key or team metadata an admin set, minus the callback credentials and the fields that steer the proxy
pub fn add_management_metadata(
    data: &mut RequestData,
    slot: MetadataField,
    management_metadata: &Metadata,
) {
    if !data.contains_key(slot.as_str()) {
        return;
    }
    let added = management_metadata
        .iter()
        .filter(|(name, _value)| {
            !CALLBACK_CONFIG_SLOTS.contains(&name.as_str())
                && !MANAGEMENT_FIELDS.contains(&name.as_str())
        })
        .map(|(name, value)| (name.clone(), value.clone()));
    let mut bucket = take_bucket(data, slot);
    if let Some(Json::Object(auth_metadata)) = bucket.get_mut(AUTH_METADATA_KEY) {
        auth_metadata.extend(added);
    } else {
        bucket.insert(AUTH_METADATA_KEY.to_owned(), Json::Object(added.collect()));
    }
    data.insert(slot.as_str().to_owned(), Json::Object(bucket));
}
