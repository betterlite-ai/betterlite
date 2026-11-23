use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::NaiveDateTime;
use rust_decimal::Decimal;

use super::membership::{
    org_member_rows, team_member_rows, write_org_member_spend, write_team_member_spend,
};
use super::{EntityTableError, EntityTableWriter, TableFuture};
use crate::key_user_writes::retry::WriteClock;
use crate::postgres_store::errors::backend;
use crate::postgres_store::session::{DbSession, PostgresStore};
use crate::spend_commit::tables::SpendTable;
use crate::spend_commit::transaction::{SpendDeadline, SpendTransactionError};

// update_many with an increment: a missing user, team, org, project, tag, access group or agent row is left missing.
pub const USER_SPEND_SQL: &str = r#"UPDATE "LiteLLM_UserTable" SET spend = spend + $2::numeric::double precision WHERE user_id = $1"#;
pub const TEAM_SPEND_SQL: &str = r#"UPDATE "LiteLLM_TeamTable" SET spend = spend + $2::numeric::double precision WHERE team_id = $1"#;
pub const ORG_SPEND_SQL: &str = r#"UPDATE "LiteLLM_OrganizationTable" SET spend = spend + $2::numeric::double precision WHERE organization_id = $1"#;
pub const PROJECT_SPEND_SQL: &str = r#"UPDATE "LiteLLM_ProjectTable" SET spend = spend + $2::numeric::double precision WHERE project_id = $1"#;
pub const TAG_SPEND_SQL: &str = r#"UPDATE "LiteLLM_TagTable" SET spend = spend + $2::numeric::double precision WHERE tag_name = $1"#;
pub const ACCESS_GROUP_SPEND_SQL: &str = r#"UPDATE "LiteLLM_ModelAccessGroupBudgetTable" SET spend = spend + $2::numeric::double precision WHERE access_group_name = $1"#;
pub const AGENT_SPEND_SQL: &str = r#"UPDATE "LiteLLM_AgentsTable" SET spend = spend + $2::numeric::double precision WHERE agent_id = $1"#;

pub const KEY_SPEND_SQL: &str = r#"UPDATE "LiteLLM_VerificationToken" SET spend = spend + $2::numeric::double precision, total_spend = total_spend + $2::numeric::double precision, last_active = $3 WHERE token = $1"#;

// update_end_user_spend: spend flushing is what creates an end user, unblocked and holding its first cost.
pub const END_USER_SPEND_SQL: &str = r#"INSERT INTO "LiteLLM_EndUserTable" (user_id, spend, blocked) VALUES ($1, $2::numeric::double precision, false) ON CONFLICT (user_id) DO UPDATE SET spend = "LiteLLM_EndUserTable".spend + EXCLUDED.spend"#;

pub struct PostgresEntityTables<'clock> {
    pub store: PostgresStore,
    pub clock: &'clock dyn WriteClock,
    pub deadline: SpendDeadline,
}

impl PostgresEntityTables<'_> {
    async fn commit_table(
        &self,
        table: SpendTable,
        increments: &BTreeMap<String, Decimal>,
        committing: &AtomicBool,
    ) -> Result<(), EntityTableError> {
        let mut client = self
            .store
            .session()
            .await
            .map_err(|source| EntityTableError::Statement { table, source })?;
        let transaction =
            client
                .transaction()
                .await
                .map_err(|source| EntityTableError::Statement {
                    table,
                    source: backend(source),
                })?;
        self.write_statements(&transaction, table, increments)
            .await?;
        committing.store(true, Ordering::SeqCst);
        transaction
            .commit()
            .await
            .map_err(|source| EntityTableError::Statement {
                table,
                source: backend(source),
            })
    }

    // The closed entity-table mapping: each table's statements in sorted id order, so every pod locks rows alike.
    async fn write_statements(
        &self,
        transaction: &DbSession<'_>,
        table: SpendTable,
        increments: &BTreeMap<String, Decimal>,
    ) -> Result<(), EntityTableError> {
        let written = match table {
            SpendTable::TeamMemberList => {
                let rows = team_member_rows(increments)
                    .map_err(|source| EntityTableError::Key { table, source })?;
                write_team_member_spend(transaction, &rows).await
            }
            SpendTable::OrgMemberList => {
                let rows = org_member_rows(increments)
                    .map_err(|source| EntityTableError::Key { table, source })?;
                write_org_member_spend(transaction, &rows).await
            }
            SpendTable::KeyList => {
                increment_keys(transaction, increments, self.clock.now().naive_utc()).await
            }
            SpendTable::UserList => increment_rows(transaction, USER_SPEND_SQL, increments).await,
            SpendTable::EndUserList => {
                increment_rows(transaction, END_USER_SPEND_SQL, increments).await
            }
            SpendTable::TeamList => increment_rows(transaction, TEAM_SPEND_SQL, increments).await,
            SpendTable::OrgList => increment_rows(transaction, ORG_SPEND_SQL, increments).await,
            SpendTable::ProjectList => {
                increment_rows(transaction, PROJECT_SPEND_SQL, increments).await
            }
            SpendTable::TagList => increment_rows(transaction, TAG_SPEND_SQL, increments).await,
            SpendTable::ModelAccessGroupList => {
                increment_rows(transaction, ACCESS_GROUP_SPEND_SQL, increments).await
            }
            SpendTable::AgentList => increment_rows(transaction, AGENT_SPEND_SQL, increments).await,
        };
        written.map_err(|source| EntityTableError::Statement {
            table,
            source: backend(source),
        })
    }
}

async fn increment_rows(
    transaction: &DbSession<'_>,
    sql: &str,
    increments: &BTreeMap<String, Decimal>,
) -> Result<(), tokio_postgres::Error> {
    let statement = transaction.prepare(sql).await?;
    for (entity_id, cost) in increments {
        transaction.execute(&statement, &[entity_id, cost]).await?;
    }
    Ok(())
}

// Keys advance spend and total_spend together and record the flush time as last_active.
async fn increment_keys(
    transaction: &DbSession<'_>,
    increments: &BTreeMap<String, Decimal>,
    last_active: NaiveDateTime,
) -> Result<(), tokio_postgres::Error> {
    let statement = transaction.prepare(KEY_SPEND_SQL).await?;
    for (token, cost) in increments {
        transaction
            .execute(&statement, &[token, cost, &last_active])
            .await?;
    }
    Ok(())
}

impl EntityTableWriter for PostgresEntityTables<'_> {
    type Error = EntityTableError;

    fn write_table<'writer>(
        &'writer self,
        table: SpendTable,
        increments: &'writer BTreeMap<String, Decimal>,
    ) -> TableFuture<'writer, EntityTableError> {
        Box::pin(async move {
            let committing = AtomicBool::new(false);
            let outcome = self
                .deadline
                .run(self.commit_table(table, increments, &committing))
                .await;
            match outcome {
                Ok(()) => Ok(()),
                Err(SpendTransactionError::Work(error)) => Err(error),
                Err(SpendTransactionError::Timeout { seconds }) => {
                    if committing.load(Ordering::SeqCst) {
                        Err(EntityTableError::CommitTimeout { table, seconds })
                    } else {
                        Err(EntityTableError::Timeout { table, seconds })
                    }
                }
            }
        })
    }
}

mod tests;
