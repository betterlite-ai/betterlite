use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_mcp::server_record::MCPServer;

use crate::key_fields::base::KeyRequestBase;
// ast-grep-ignore: no-long-names
use crate::object_permissions::update::handle_update_object_permission_common;

use super::Row;
use super::error::KeyPatchError;

// ast-grep-ignore: no-long-names - _handle_update_object_permission: the request's permission row is persisted and swapped for its id.
pub async fn handle_update_object_permission(
    data: &mut Row,
    existing_object_permission_id: Option<&str>,
    client: PrismaClient<'_>,
    registry: &[&MCPServer],
) -> Result<(), KeyPatchError> {
    if !data.contains_key("object_permission") {
        return Ok(());
    }
    // ast-grep-ignore: no-long-names - the shared permission write keeps its name from the source
    let updated = handle_update_object_permission_common(
        data,
        existing_object_permission_id,
        Some(client),
        registry,
    )
    .await?;
    if let Some(object_permission_id) = updated {
        tracing::debug!(object_permission_id, "updated object_permission_id");
        data.insert(
            String::from("object_permission_id"),
            JsonData::String(object_permission_id),
        );
    }
    Ok(())
}

// is_different_team: an omitted team changes nothing, and a team against an unset stored team always differs.
#[must_use]
pub fn is_different_team(request_team: Option<&str>, existing_team: Option<&str>) -> bool {
    let Some(team) = request_team else {
        return false;
    };
    existing_team.is_none_or(|existing| existing != team)
}

// _resolve_token_to_update: the request's key wins even when empty, then the stored token, then the 404.
pub fn resolve_token_to_update<'token>(
    request_key: Option<&'token str>,
    existing_token: Option<&'token str>,
) -> Result<&'token str, KeyPatchError> {
    if let Some(key) = request_key {
        return Ok(key);
    }
    existing_token.ok_or(KeyPatchError::KeyNotFound)
}

// ast-grep-ignore: no-one-line-functions - _require_prisma_client: a write without a database is the 500 the source raises.
pub fn require_prisma_client(
    client: Option<PrismaClient<'_>>,
) -> Result<PrismaClient<'_>, KeyPatchError> {
    client.ok_or(KeyPatchError::DatabaseNotConnected)
}

// _requested_end_user_budget_id: the top-level id wins; a supplied map without a usable id clears the value.
#[must_use]
pub fn requested_end_user_budget_id(request: &KeyRequestBase) -> Option<String> {
    if let Some(Some(budget_id)) = &request.key_fields.end_user_budget_id {
        return Some(budget_id.clone());
    }
    let Some(Some(metadata)) = &request.base.base.metadata else {
        return None;
    };
    match metadata.0.get("end_user_budget_id") {
        Some(JsonData::String(text)) if !text.is_empty() => Some(text.clone()),
        Some(
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::String(_)
            | JsonData::Array(_)
            | JsonData::Object(_),
        )
        | None => Some(String::new()),
    }
}

// _general_settings: the loaded globals must be a JSON object; the loader itself belongs to the config crate.
pub fn general_settings(value: &JsonData) -> Result<Row, KeyPatchError> {
    match value {
        JsonData::Object(settings) => Ok(settings.clone()),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => Err(KeyPatchError::GeneralSettings),
    }
}
