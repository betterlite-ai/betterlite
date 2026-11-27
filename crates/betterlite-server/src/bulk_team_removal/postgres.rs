use std::time::Duration;

use async_trait::async_trait;
use betterlite_auth::auth_key_lookup::jwt::JwtMappingSource;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::key_repository::decode::decode_token_record;
use betterlite_db::member_removal_identity::identity::MemberIdentity;
use betterlite_db::postgres_store::errors::backend;
use betterlite_db::postgres_store::session::{DbSession, PostgresStore};
use betterlite_db::team_repository::repository::{TeamDatabase, TeamRepository};
use betterlite_db::team_rows::member::Member;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_db::user_repository::projection::decode_user_record;
use tokio::time::timeout;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::error::BulkRemovalError;
use super::removal::{RemovalTables, TeamRemoval, TeamRemovalSession, remove_members_from_team};
use crate::access_group_sync::team::TEAM_ADVISORY_LOCK_SQL;
use crate::audit_events::identity::CallerIdentity;
use crate::key_archive::clock::ArchiveClock;

// ast-grep-ignore: no-long-names
use crate::key_archive::persist_deleted_verification_tokens;

// _BATCH_TX_TIMEOUT: prisma_client.tx measures the whole callback, and a timeout rolls the transaction back
pub const BATCH_TX_TIMEOUT: Duration = Duration::from_secs(60);

const REQUESTED_USERS_SQL: &str = r#"SELECT row_to_json(entry)::TEXT FROM "LiteLLM_UserTable" AS entry WHERE entry."user_id" = ANY($1::TEXT[]) OR entry."user_email" = ANY($2::TEXT[])"#;
const TEAM_KEYS_SQL: &str = r#"SELECT row_to_json(entry)::TEXT FROM "LiteLLM_VerificationToken" AS entry WHERE entry."team_id" = $1 AND entry."user_id" = ANY($2::TEXT[])"#;
const WRITE_ROSTER_SQL: &str = r#"UPDATE "LiteLLM_TeamTable" SET "members_with_roles" = to_jsonb($2::TEXT), "updated_at" = (NOW() AT TIME ZONE 'UTC') WHERE "team_id" = $1"#;
const WRITE_TEAMS_SQL: &str = r#"UPDATE "LiteLLM_UserTable" SET "teams" = $2::TEXT[], "updated_at" = (NOW() AT TIME ZONE 'UTC') WHERE "user_id" = $1"#;
const DELETE_MEMBERSHIPS_SQL: &str =
    r#"DELETE FROM "LiteLLM_TeamMembership" WHERE "team_id" = $1 AND "user_id" = ANY($2::TEXT[])"#;
const DELETE_KEYS_SQL: &str = r#"DELETE FROM "LiteLLM_VerificationToken" WHERE "team_id" = $1 AND "user_id" = ANY($2::TEXT[])"#;

pub struct PostgresTeamRemoval<'deps> {
    pub store: &'deps PostgresStore,
    pub clock: &'deps dyn ArchiveClock,
    pub timeout: Duration,
}

struct PostgresRemovalTables<'tables, 'client> {
    session: &'tables DbSession<'client>,
    clock: &'tables dyn ArchiveClock,
}

#[async_trait]
impl TeamRemovalSession for PostgresTeamRemoval<'_> {
    async fn find_team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, BulkRemovalError> {
        let repository = TeamRepository {
            database: TeamDatabase {
                db: self.store,
                writer_db: self.store,
            },
            use_writer: false,
        };
        repository
            .find_by_id(team_id)
            .await
            .map_err(BulkRemovalError::from)
    }

    async fn remove_members(
        &self,
        team_id: &str,
        members: &[MemberIdentity],
        caller: &CallerIdentity,
        jwt: &dyn JwtMappingSource,
    ) -> Result<TeamRemoval, BulkRemovalError> {
        let mut client = self.store.session().await?;
        let transaction = client.transaction().await.map_err(backend)?;
        let tables = PostgresRemovalTables {
            session: &transaction,
            clock: self.clock,
        };
        let removal = match timeout(
            self.timeout,
            remove_members_from_team(&tables, team_id, members, caller, jwt),
        )
        .await
        {
            Ok(outcome) => outcome?,
            Err(_) => {
                return Err(BulkRemovalError::Timeout {
                    seconds: self.timeout.as_secs(),
                });
            }
        };
        transaction.commit().await.map_err(backend)?;
        Ok(removal)
    }
}

#[async_trait]
impl RemovalTables for PostgresRemovalTables<'_, '_> {
    async fn lock_team(&self, team_id: &str) -> Result<(), BulkRemovalError> {
        self.session
            .query(TEAM_ADVISORY_LOCK_SQL, &[&team_id])
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn roster(&self, team_id: &str) -> Result<Option<Vec<Member>>, BulkRemovalError> {
        TeamRepository::get_members_with_roles_locked(self.session, team_id)
            .await
            .map_err(BulkRemovalError::from)
    }

    async fn users(
        &self,
        user_ids: &[String],
        user_emails: &[String],
    ) -> Result<Vec<LiteLLMUserTable>, BulkRemovalError> {
        let parameters: [&(dyn ToSql + Sync); 2] = [&user_ids, &user_emails];
        let rows = self
            .session
            .query(REQUESTED_USERS_SQL, &parameters)
            .await
            .map_err(backend)?;
        rows.iter().map(user_row).collect()
    }

    async fn tokens(
        &self,
        team_id: &str,
        user_ids: &[String],
    ) -> Result<Vec<LiteLLMVerificationToken>, BulkRemovalError> {
        let parameters: [&(dyn ToSql + Sync); 2] = [&team_id, &user_ids];
        let rows = self
            .session
            .query(TEAM_KEYS_SQL, &parameters)
            .await
            .map_err(backend)?;
        rows.iter().map(key_row).collect()
    }

    async fn write_roster(&self, team_id: &str, kept: &[Member]) -> Result<(), BulkRemovalError> {
        let roster = serde_json::to_string(kept)?;
        self.session
            .execute(WRITE_ROSTER_SQL, &[&team_id, &roster])
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn write_user_teams(
        &self,
        user_id: &str,
        teams: &[String],
    ) -> Result<(), BulkRemovalError> {
        self.session
            .execute(WRITE_TEAMS_SQL, &[&user_id, &teams])
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn delete_memberships(
        &self,
        team_id: &str,
        user_ids: &[String],
    ) -> Result<(), BulkRemovalError> {
        self.session
            .execute(DELETE_MEMBERSHIPS_SQL, &[&team_id, &user_ids])
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn archive_tokens(
        &self,
        keys: &[LiteLLMVerificationToken],
        caller: &CallerIdentity,
    ) -> Result<(), BulkRemovalError> {
        // ast-grep-ignore: no-long-names
        persist_deleted_verification_tokens(self.session, keys, caller, None, self.clock)
            .await
            .map_err(BulkRemovalError::from)
    }

    async fn delete_tokens(
        &self,
        team_id: &str,
        user_ids: &[String],
    ) -> Result<(), BulkRemovalError> {
        self.session
            .execute(DELETE_KEYS_SQL, &[&team_id, &user_ids])
            .await
            .map_err(backend)?;
        Ok(())
    }
}

fn user_row(row: &Row) -> Result<LiteLLMUserTable, BulkRemovalError> {
    let text: String = row.try_get(0).map_err(backend)?;
    decode_user_record(text.as_bytes())?.ok_or(BulkRemovalError::MissingRow)
}

fn key_row(row: &Row) -> Result<LiteLLMVerificationToken, BulkRemovalError> {
    let text: String = row.try_get(0).map_err(backend)?;
    decode_token_record(text.as_bytes()).map_err(BulkRemovalError::from)
}
