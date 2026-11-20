use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::user_repository::projection::user_record_to_model;

use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

use super::lookup::{BlockFailure, UserLookup, UserLookupRequest};
use super::rows::UserCreateData;
use super::store::UserStore;

// The user columns a create writes; every other merged key stays out of the row.
const USER_COLUMNS: [&str; 26] = [
    "user_id",
    "user_alias",
    "team_id",
    "sso_user_id",
    "organization_id",
    "object_permission_id",
    "password_reset_required",
    "last_breach_check_at",
    "teams",
    "user_role",
    "max_budget",
    "spend",
    "user_email",
    "models",
    "metadata",
    "max_parallel_requests",
    "tpm_limit",
    "rpm_limit",
    "budget_duration",
    "budget_reset_at",
    "allowed_cache_controls",
    "policies",
    "model_spend",
    "model_max_budget",
    "created_at",
    "updated_at",
];

// The user_id_upsert branch: the merged params become a row, then the admin enrollment runs.
pub(super) async fn create_user(
    lookup: &UserLookup,
    store: &dyn UserStore,
    request: &UserLookupRequest<'_>,
    user_id: &str,
) -> Result<JsonData, BlockFailure> {
    let mut params: BTreeMap<String, JsonData> = BTreeMap::new();
    params.insert("user_id".to_owned(), JsonData::String(user_id.to_owned()));
    if let Some(user_email) = request.user_email {
        params.insert(
            "user_email".to_owned(),
            JsonData::String(user_email.to_owned()),
        );
    }
    if let Some(defaults) = lookup.defaults.as_ref() {
        for (key, value) in &defaults.values {
            if key == "teams" || key == "available_teams" {
                continue;
            }
            params.insert(key.clone(), value.clone());
        }
    }
    apply_budget_reset(lookup, &mut params)?;
    let data = create_data(&params)?;
    let record = store.create(data).await.map_err(BlockFailure::Store)?;
    let auth = proxy_admin_auth()?;
    lookup
        .enrollment
        .enroll(user_id, request.user_email, &auth)
        .await
        .map_err(BlockFailure::Port)?;
    Ok(record)
}

// get_budget_reset_time(budget_duration=...), only when the merged params leave the reset unset
fn apply_budget_reset(
    lookup: &UserLookup,
    params: &mut BTreeMap<String, JsonData>,
) -> Result<(), BlockFailure> {
    let Some(JsonData::String(budget_duration)) = params.get("budget_duration") else {
        return Ok(());
    };
    if params
        .get("budget_reset_at")
        .is_some_and(|value| !matches!(value, JsonData::Null))
    {
        return Ok(());
    }
    let reset = lookup
        .budget_reset
        .reset_time(budget_duration)
        .map_err(BlockFailure::Port)?;
    let wire = serde_json::to_value(&reset).map_err(BlockFailure::Serialize)?;
    let value = serde_json::from_value(wire).map_err(BlockFailure::Serialize)?;
    params.insert("budget_reset_at".to_owned(), value);
    Ok(())
}

// The merged create params validate against B720's user model before any row is written
fn create_data(params: &BTreeMap<String, JsonData>) -> Result<UserCreateData, BlockFailure> {
    let record = JsonData::Object(params.clone());
    let user = user_record_to_model(Some(&record))?.ok_or(BlockFailure::NoRecord)?;
    let wire = serde_json::to_value(&user).map_err(BlockFailure::Serialize)?;
    let dump: BTreeMap<String, JsonData> =
        serde_json::from_value(wire).map_err(BlockFailure::Serialize)?;
    let mut fields = BTreeMap::new();
    for key in params.keys() {
        if !USER_COLUMNS.contains(&key.as_str()) {
            continue;
        }
        if let Some(value) = dump.get(key) {
            fields.insert(key.clone(), value.clone());
        }
    }
    UserCreateData::new(fields).map_err(BlockFailure::Serialize)
}

// UserAPIKeyAuth(user_role=LitellmUserRoles.PROXY_ADMIN): the admin identity enrollment runs as
fn proxy_admin_auth() -> Result<UserAPIKeyAuth, BlockFailure> {
    let mut fields = BTreeMap::new();
    fields.insert(
        "user_role".to_owned(),
        JsonData::String("proxy_admin".to_owned()),
    );
    Ok(UserAPIKeyAuth::construct(AuthInput::Dict(fields))?)
}
