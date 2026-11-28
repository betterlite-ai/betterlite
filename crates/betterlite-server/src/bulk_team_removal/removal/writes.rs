use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::team_rows::member::Member;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use secrecy::ExposeSecret as _;
use serde::Serialize;

use super::{DeletedKeys, RemovalTables, Selection};
use crate::bulk_team_removal::error::BulkRemovalError;

#[derive(Serialize)]
struct RosterRow<'row> {
    team_id: &'row str,
    members_with_roles: &'row [Member],
}

pub(super) async fn keys_for(
    tables: &dyn RemovalTables,
    team_id: &str,
    cleanup_ids: &[String],
) -> Result<Vec<LiteLLMVerificationToken>, BulkRemovalError> {
    if cleanup_ids.is_empty() {
        return Ok(Vec::new());
    }
    tables.tokens(team_id, cleanup_ids).await
}

pub(crate) fn key_tokens(keys: &[LiteLLMVerificationToken]) -> Vec<String> {
    keys.iter()
        .filter_map(|key| {
            key.token
                .as_ref()
                .map(|token| token.expose_secret().to_owned())
        })
        .collect()
}

pub(super) async fn write_roster_and_memberships(
    tables: &dyn RemovalTables,
    team_id: &str,
    selection: &Selection,
) -> Result<(), BulkRemovalError> {
    if !selection.removed.is_empty() {
        tables.write_roster(team_id, &selection.kept).await?;
    }
    for row in &selection.stale_rows {
        let teams: Vec<String> = row
            .teams
            .iter()
            .filter(|team| *team != team_id)
            .cloned()
            .collect();
        tables.write_user_teams(&row.user_id, &teams).await?;
    }
    tables
        .delete_memberships(team_id, &selection.cleanup_ids)
        .await
}

pub(super) async fn archive_and_delete_keys(
    tables: &dyn RemovalTables,
    team_id: &str,
    deleted: DeletedKeys<'_>,
) -> Result<(), BulkRemovalError> {
    if deleted.keys.is_empty() {
        return Ok(());
    }
    tables.archive_tokens(deleted.keys, deleted.caller).await?;
    tables.delete_tokens(team_id, deleted.cleanup_ids).await
}

pub(super) fn kept_team(
    team_id: &str,
    kept: &[Member],
) -> Result<LiteLLMTeamTable, BulkRemovalError> {
    let text = serde_json::to_string(&RosterRow {
        team_id,
        members_with_roles: kept,
    })?;
    serde_json::from_str(&text).map_err(BulkRemovalError::Encode)
}
