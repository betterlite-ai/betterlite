use std::collections::BTreeSet;

use async_trait::async_trait;
use betterlite_auth::auth_key_lookup::jwt::JwtMappingSource;

// ast-grep-ignore: no-long-names
use betterlite_auth::auth_key_lookup::jwt::get_jwt_key_mapping_cache_keys_for_tokens;

use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::member_removal_identity::identity::MemberIdentity;
use betterlite_db::team_rows::member::Member;
use betterlite_db::team_rows::team::LiteLLMTeamTable;

use self::selection::select_members;
use self::writes::{
    archive_and_delete_keys, kept_team, key_tokens, keys_for, write_roster_and_memberships,
};
use super::error::BulkRemovalError;
use crate::audit_events::identity::CallerIdentity;
use crate::team_problems::team_not_found;

mod selection;
mod writes;

#[async_trait]
pub trait RemovalTables: Send + Sync {
    async fn lock_team(&self, team_id: &str) -> Result<(), BulkRemovalError>;

    async fn roster(&self, team_id: &str) -> Result<Option<Vec<Member>>, BulkRemovalError>;

    async fn users(
        &self,
        user_ids: &[String],
        user_emails: &[String],
    ) -> Result<Vec<LiteLLMUserTable>, BulkRemovalError>;

    async fn tokens(
        &self,
        team_id: &str,
        user_ids: &[String],
    ) -> Result<Vec<LiteLLMVerificationToken>, BulkRemovalError>;

    async fn write_roster(&self, team_id: &str, kept: &[Member]) -> Result<(), BulkRemovalError>;

    async fn write_user_teams(
        &self,
        user_id: &str,
        teams: &[String],
    ) -> Result<(), BulkRemovalError>;

    async fn delete_memberships(
        &self,
        team_id: &str,
        user_ids: &[String],
    ) -> Result<(), BulkRemovalError>;

    async fn archive_tokens(
        &self,
        keys: &[LiteLLMVerificationToken],
        caller: &CallerIdentity,
    ) -> Result<(), BulkRemovalError>;

    async fn delete_tokens(
        &self,
        team_id: &str,
        user_ids: &[String],
    ) -> Result<(), BulkRemovalError>;
}

// the transaction opens with a team lock and a fresh roster read, so the removal is one committed unit
#[async_trait]
pub trait TeamRemovalSession: Send + Sync {
    async fn find_team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, BulkRemovalError>;

    async fn remove_members(
        &self,
        team_id: &str,
        members: &[MemberIdentity],
        caller: &CallerIdentity,
        jwt: &dyn JwtMappingSource,
    ) -> Result<TeamRemoval, BulkRemovalError>;
}

#[derive(Debug, Clone)]
pub struct TeamRemoval {
    pub team: LiteLLMTeamTable,
    pub removed: BTreeSet<String>,
    pub matched: BTreeSet<usize>,
    pub deleted_keys: Vec<LiteLLMVerificationToken>,
    pub jwt_mapping_cache_keys: Vec<String>,
}

struct Selection {
    removed: Vec<Member>,
    kept: Vec<Member>,
    stale_rows: Vec<LiteLLMUserTable>,
    cleanup_ids: Vec<String>,
    matched: BTreeSet<usize>,
}

struct DeletedKeys<'keys> {
    keys: &'keys [LiteLLMVerificationToken],
    cleanup_ids: &'keys [String],
    caller: &'keys CallerIdentity,
}

impl TeamRemoval {
    // the token is the verification table's primary key, so a stored key row always carries one
    #[must_use]
    pub fn deleted_key_tokens(&self) -> Vec<String> {
        key_tokens(&self.deleted_keys)
    }
}

pub async fn remove_members_from_team(
    tables: &dyn RemovalTables,
    team_id: &str,
    members: &[MemberIdentity],
    caller: &CallerIdentity,
    jwt: &dyn JwtMappingSource,
) -> Result<TeamRemoval, BulkRemovalError> {
    tables.lock_team(team_id).await?;
    let Some(roster) = tables.roster(team_id).await? else {
        return Err(BulkRemovalError::Problem(team_not_found(team_id)));
    };
    let selection = select_members(tables, team_id, members, roster).await?;
    let keys = keys_for(tables, team_id, &selection.cleanup_ids).await?;
    // ast-grep-ignore: no-long-names
    let jwt_mapping_cache_keys =
        get_jwt_key_mapping_cache_keys_for_tokens(&key_tokens(&keys), jwt).await?;
    write_roster_and_memberships(tables, team_id, &selection).await?;
    archive_and_delete_keys(
        tables,
        team_id,
        DeletedKeys {
            keys: &keys,
            cleanup_ids: &selection.cleanup_ids,
            caller,
        },
    )
    .await?;
    Ok(TeamRemoval {
        team: kept_team(team_id, &selection.kept)?,
        removed: selection.cleanup_ids.into_iter().collect(),
        matched: selection.matched,
        deleted_keys: keys,
        jwt_mapping_cache_keys,
    })
}
