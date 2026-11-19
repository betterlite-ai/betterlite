use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_rows::access_group::AccessGroupTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use serde_json::{Value, json};

use super::{Store, field, invalidation, lock, number, text};

pub(super) async fn management(
    store: &Arc<Store>,
    team: &JsonData,
) -> Result<Value, Box<HelperError>> {
    let row = LiteLLMTeamTableCachedObj::try_from(team.clone()).or_value_error()?;
    let key = format!("team_id:{}", row.team.fields.base.team_id);
    let invalidation = invalidation(store, 0.0_f64);
    invalidation
        .cache_management_object(&key, &row)
        .await
        .or_value_error()?;
    let stored = store.entry(&key);
    Ok(json!({
        "management_model_type_and_ttl": {
            "key": key,
            "ttl": stored.1,
            "team_id": text(&stored.0, "team_id"),
            "team_alias": text(&stored.0, "team_alias"),
        },
        "management_logging_argument_unused": true,
    }))
}

pub(super) async fn team_refresh(
    store: &Arc<Store>,
    team_id: &str,
    team: &JsonData,
    stale_alias: &JsonData,
) -> Result<Value, Box<HelperError>> {
    let mut row = LiteLLMTeamTableCachedObj::try_from(team.clone()).or_value_error()?;
    let alias = row.team.fields.base.team_alias.clone();
    let alias_key = alias.as_ref().map(|alias| format!("team_alias:{alias}"));
    if let Some(alias_key) = alias_key.as_ref() {
        store.seed(alias_key, stale_alias);
    }
    let was_none = row.last_refreshed_at.is_none();
    let invalidation = invalidation(store, 0.0_f64);
    invalidation
        .cache_team_object(team_id, &mut row)
        .await
        .or_value_error()?;
    let key = format!("team_id:{team_id}");
    let stored = store.entry(&key);
    let alias_present = alias_key
        .as_ref()
        .is_some_and(|alias_key| lock(&store.entries).contains_key(alias_key));
    let stored_timestamp = number(&stored.0, "last_refreshed_at");
    Ok(json!({
        "team_id_and_alias_keys": {
            "team_key": key,
            "team_key_ttl": stored.1,
            "alias_key": alias_key,
            "alias_present": alias_present,
        },
        "team_timestamp_mutates_input": {
            "input_was_none": was_none,
            "input_now_is_none": row.last_refreshed_at.is_none(),
            "stored_has_timestamp": stored_timestamp.is_some(),
            "stored_equals_input": stored_timestamp == row.last_refreshed_at,
        },
    }))
}

pub(super) async fn key_refresh(
    store: &Arc<Store>,
    hashed_token: &str,
    rehash_candidate: &str,
    auth: &BTreeMap<String, JsonData>,
) -> Result<Value, Box<HelperError>> {
    let mut original = UserAPIKeyAuth::construct(AuthInput::Dict(auth.clone())).or_value_error()?;
    let invalidation = invalidation(store, 0.0_f64);
    invalidation
        .cache_key_object(hashed_token, &mut original)
        .await
        .or_value_error()?;
    let original_wire = serde_json::to_value(&original).or_value_error()?;
    let original_timestamp = original_wire
        .get("last_refreshed_at")
        .and_then(Value::as_f64);
    let original_route = original_wire
        .get("request_route")
        .cloned()
        .unwrap_or(Value::Null);
    let stored = store.entry(hashed_token);
    let stored_timestamp = number(&stored.0, "last_refreshed_at");
    Ok(json!({
        "key_timestamp_and_request_field_copy": {
            "stored_ttl": stored.1,
            "stored_request_route_is_null": matches!(
                field(&stored.0, "request_route"),
                Some(Json::Null) | None
            ),
            "stored_has_timestamp": stored_timestamp.is_some(),
        },
        "key_hash_not_recomputed": {
            "supplied_key_present": lock(&store.entries).contains_key(hashed_token),
            "rehash_key_present": lock(&store.entries).contains_key(rehash_candidate),
        },
        "key_copy_retains_original_request_fields": {
            "original_request_route": original_route,
            "original_last_refreshed_at_matches_stored": original_timestamp == stored_timestamp,
        },
    }))
}

pub(super) async fn access_group(
    store: &Arc<Store>,
    access_group_ttl: f64,
    access_group_id: &str,
    table: &JsonData,
) -> Result<Value, Box<HelperError>> {
    let wire = serde_json::to_value(table).or_value_error()?;
    let row: AccessGroupTable = serde_json::from_value(wire).or_value_error()?;
    let invalidation = invalidation(store, access_group_ttl);
    invalidation
        .cache_access_object(access_group_id, &row)
        .await
        .or_value_error()?;
    let key = format!("access_group_id:{access_group_id}");
    let stored = store.entry(&key);
    Ok(json!({
        "access_group_key_and_independent_ttl": {
            "key": key,
            "ttl": stored.1,
            "access_group_id": text(&stored.0, "access_group_id"),
            "access_group_name": text(&stored.0, "access_group_name"),
        },
    }))
}
