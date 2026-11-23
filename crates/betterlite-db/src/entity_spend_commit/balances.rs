use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use rust_decimal::Decimal;

use super::membership::{MemberSpendWrite, org_member_rows, team_member_rows, write_member_spend};
use super::{EntityTableError, EntityTableWriter, TableFuture};
use crate::key_user_writes::retry::WriteClock;
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::{DbSession, PostgresStore};
use crate::spend_commit::tables::SpendTable;
use crate::spend_commit::transaction::{SpendDeadline, SpendTransactionError};

// update_many with an increment: a missing user, team, org, project, tag, access group or agent row is left missing, and @updatedAt stamps $3.
pub const USER_SPEND_SQL: &str = r#"UPDATE "LiteLLM_UserTable" SET spend = spend + $2::numeric::double precision, updated_at = $3 WHERE user_id = $1"#;
pub const TEAM_SPEND_SQL: &str = r#"UPDATE "LiteLLM_TeamTable" SET spend = spend + $2::numeric::double precision, updated_at = $3 WHERE team_id = $1"#;
pub const ORG_SPEND_SQL: &str = r#"UPDATE "LiteLLM_OrganizationTable" SET spend = spend + $2::numeric::double precision, updated_at = $3 WHERE organization_id = $1"#;
pub const PROJECT_SPEND_SQL: &str = r#"UPDATE "LiteLLM_ProjectTable" SET spend = spend + $2::numeric::double precision, updated_at = $3 WHERE project_id = $1"#;
pub const TAG_SPEND_SQL: &str = r#"UPDATE "LiteLLM_TagTable" SET spend = spend + $2::numeric::double precision, updated_at = $3 WHERE tag_name = $1"#;
pub const ACCESS_GROUP_SPEND_SQL: &str = r#"UPDATE "LiteLLM_ModelAccessGroupBudgetTable" SET spend = spend + $2::numeric::double precision, updated_at = $3 WHERE access_group_name = $1"#;
pub const AGENT_SPEND_SQL: &str = r#"UPDATE "LiteLLM_AgentsTable" SET spend = spend + $2::numeric::double precision, updated_at = $3 WHERE agent_id = $1"#;

pub const KEY_SPEND_SQL: &str = r#"UPDATE "LiteLLM_VerificationToken" SET spend = spend + $2::numeric::double precision, total_spend = total_spend + $2::numeric::double precision, last_active = $3, updated_at = $3 WHERE token = $1"#;

// update_end_user_spend: spend flushing is what creates an end user, unblocked and holding its first cost.
pub const END_USER_SPEND_SQL: &str = r#"INSERT INTO "LiteLLM_EndUserTable" (user_id, spend, blocked) VALUES ($1, $2::numeric::double precision, false) ON CONFLICT (user_id) DO UPDATE SET spend = "LiteLLM_EndUserTable".spend + EXCLUDED.spend"#;

pub struct PostgresEntityTables<'clock> {
    pub store: PostgresStore,
    pub clock: &'clock dyn WriteClock,
    pub deadline: SpendDeadline,
}

// What one table's transaction writes.
enum TableWrite<'rows> {
    // a table without an updated_at column: the statement it issues and the rows it binds
    Rows(&'static str, &'rows BTreeMap<String, Decimal>),
    // a table whose update stamps the flush moment into $3: updated_at, and for keys last_active as well
    Stamped(&'static str, &'rows BTreeMap<String, Decimal>),
    Members(MemberSpendWrite<'rows>),
}

impl PostgresEntityTables<'_> {
    // One table's transaction: its rows decoded, its statements and its COMMIT under the source's one deadline.
    async fn commit_rows(
        &self,
        table: SpendTable,
        increments: &BTreeMap<String, Decimal>,
    ) -> Result<(), EntityTableError> {
        let write = match table {
            SpendTable::UserList => TableWrite::Stamped(USER_SPEND_SQL, increments),
            SpendTable::EndUserList => TableWrite::Rows(END_USER_SPEND_SQL, increments),
            SpendTable::KeyList => TableWrite::Stamped(KEY_SPEND_SQL, increments),
            SpendTable::TeamList => TableWrite::Stamped(TEAM_SPEND_SQL, increments),
            SpendTable::TeamMemberList => {
                let rows = team_member_rows(increments)
                    .map_err(|source| EntityTableError::Key { table, source })?;
                TableWrite::Members(MemberSpendWrite::Team(rows))
            }
            SpendTable::OrgList => TableWrite::Stamped(ORG_SPEND_SQL, increments),
            SpendTable::OrgMemberList => {
                let rows = org_member_rows(increments)
                    .map_err(|source| EntityTableError::Key { table, source })?;
                TableWrite::Members(MemberSpendWrite::Organization(rows))
            }
            SpendTable::ProjectList => TableWrite::Stamped(PROJECT_SPEND_SQL, increments),
            SpendTable::TagList => TableWrite::Stamped(TAG_SPEND_SQL, increments),
            SpendTable::ModelAccessGroupList => {
                TableWrite::Stamped(ACCESS_GROUP_SPEND_SQL, increments)
            }
            SpendTable::AgentList => TableWrite::Stamped(AGENT_SPEND_SQL, increments),
        };
        let committing = AtomicBool::new(false);
        match self.deadline.run(self.run_table(write, &committing)).await {
            Ok(()) => Ok(()),
            Err(SpendTransactionError::Work(source)) => {
                Err(EntityTableError::Statement { table, source })
            }
            // a deadline that fires after COMMIT was sent may have applied, so that table is not requeue-safe
            Err(SpendTransactionError::Timeout { seconds }) => {
                if committing.load(Ordering::SeqCst) {
                    Err(EntityTableError::CommitTimeout { table, seconds })
                } else {
                    Err(EntityTableError::Timeout { table, seconds })
                }
            }
        }
    }

    // The session, the table's statements, the mark and the COMMIT itself.
    async fn run_table(
        &self,
        write: TableWrite<'_>,
        committing: &AtomicBool,
    ) -> Result<(), DbError> {
        let mut client = self.store.session().await?;
        let transaction = client.transaction().await.map_err(backend)?;
        execute_table(&transaction, self.clock, write)
            .await
            .map_err(backend)?;
        committing.store(true, Ordering::SeqCst);
        transaction.commit().await.map_err(backend)
    }
}

// One table's statements inside its transaction, each issued in sorted id order so every pod locks rows alike.
async fn execute_table(
    transaction: &DbSession<'_>,
    clock: &dyn WriteClock,
    write: TableWrite<'_>,
) -> Result<(), tokio_postgres::Error> {
    match write {
        TableWrite::Rows(sql, rows) => increment_rows(transaction, sql, rows, None).await,
        TableWrite::Stamped(sql, rows) => increment_rows(transaction, sql, rows, Some(clock)).await,
        TableWrite::Members(members) => write_member_spend(transaction, members, clock).await,
    }
}

// One prepared statement per sorted id, and a stamped table reads the clock once per statement as Prisma's request_now does.
async fn increment_rows(
    transaction: &DbSession<'_>,
    sql: &str,
    increments: &BTreeMap<String, Decimal>,
    clock: Option<&dyn WriteClock>,
) -> Result<(), tokio_postgres::Error> {
    let statement = transaction.prepare(sql).await?;
    for (entity_id, cost) in increments {
        match clock {
            Some(stamping) => {
                let at = stamping.now().naive_utc();
                transaction
                    .execute(&statement, &[entity_id, cost, &at])
                    .await?;
            }
            None => {
                transaction.execute(&statement, &[entity_id, cost]).await?;
            }
        }
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
        Box::pin(self.commit_rows(table, increments))
    }
}

mod tests;
