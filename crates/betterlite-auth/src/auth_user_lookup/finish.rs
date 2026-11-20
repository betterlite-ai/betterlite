use betterlite_db::budget_permission_rows::{JsonData, OrganizationMembership};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::user_repository::projection::user_record_to_model;

use super::lookup::{BlockFailure, UserLookup};

// LiteLLM_UserTable.model_validate(dict(response)), then the null-email backfill, cache and cadence
pub(super) async fn finish(
    lookup: &UserLookup,
    record: JsonData,
    email: Option<&str>,
    requested_id: &str,
) -> Result<LiteLLMUserTable, BlockFailure> {
    let record = normalize_memberships(record)?;
    let user = user_record_to_model(Some(&record))?.ok_or(BlockFailure::NoRecord)?;
    let user = backfill(lookup, user, email).await?;
    let ttl = lookup.cache.management_ttl();
    lookup
        .cache
        .write(requested_id, &user, ttl)
        .await
        .map_err(BlockFailure::Cache)?;
    let wire = serde_json::to_value(&user).map_err(BlockFailure::Serialize)?;
    let dump = serde_json::from_value(wire).map_err(BlockFailure::Serialize)?;
    lookup.cadence.update(
        &format!("user_id:{requested_id}"),
        Some(JsonData::Object(dump)),
    );
    Ok(user)
}

// _backfill_null_user_email: skipped without an input email, a stored email or a database
pub(super) async fn backfill(
    lookup: &UserLookup,
    user: LiteLLMUserTable,
    email: Option<&str>,
) -> Result<LiteLLMUserTable, BlockFailure> {
    let Some(email) = email else {
        return Ok(user);
    };
    if user.user_email.is_some() {
        return Ok(user);
    }
    let Some(store) = lookup.store.as_deref() else {
        return Ok(user);
    };
    store
        .backfill_null_email(&user.user_id, email)
        .await
        .map_err(BlockFailure::Store)?;
    let Some(reread) = store
        .reread_for_backfill(&user.user_id)
        .await
        .map_err(BlockFailure::Store)?
    else {
        return Ok(user);
    };
    let mut updated = user;
    updated.user_email = reread.user_email;
    let canonical = updated.user_id.clone();
    let ttl = lookup.cache.management_ttl();
    lookup
        .cache
        .write(&canonical, &updated, ttl)
        .await
        .map_err(BlockFailure::Cache)?;
    Ok(updated)
}

// The membership include: for a nonempty sequence, null entries drop and each remaining entry validates
fn normalize_memberships(record: JsonData) -> Result<JsonData, BlockFailure> {
    let JsonData::Object(mut fields) = record else {
        return Ok(record);
    };
    let Some(JsonData::Array(memberships)) = fields.get("organization_memberships") else {
        return Ok(JsonData::Object(fields));
    };
    if memberships.is_empty() {
        return Ok(JsonData::Object(fields));
    }
    let kept: Vec<JsonData> = memberships
        .iter()
        .filter(|entry| !matches!(entry, JsonData::Null))
        .cloned()
        .collect();
    let wire = serde_json::to_value(&kept).map_err(BlockFailure::Membership)?;
    serde_json::from_value::<Vec<OrganizationMembership>>(wire)
        .map_err(BlockFailure::Membership)?;
    fields.insert("organization_memberships".to_owned(), JsonData::Array(kept));
    Ok(JsonData::Object(fields))
}
