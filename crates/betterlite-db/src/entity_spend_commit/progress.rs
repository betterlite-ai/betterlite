use std::collections::BTreeMap;
use std::error::Error as StdError;

use rust_decimal::Decimal;
use thiserror::Error;

use super::EntityTableWriter;
use super::membership::{MemberKeyError, team_member_key};
use crate::budget_reset_policy::cache_invalidation::ResetCache;
use crate::spend_commit::policy::{
    SpendFailure, SpendRetry, SpendRetryPolicy, SpendUpdateFailure, handle_spend_update_failure,
};
use crate::spend_commit::tables::{SpendTable, SpendTableTransactions};

#[derive(Debug, Error)]
pub enum EntityCommitError<Write, Cache> {
    #[error("the entity spend commit failed")]
    Write(#[source] Write),
    #[error("the team membership increments hold a malformed key")]
    TeamMemberKey(#[source] MemberKeyError),
    #[error("evicting the cache entries the {} commit changed failed", .table.name())]
    Cache {
        table: SpendTable,
        #[source]
        source: Cache,
    },
}

impl<Write, Cache> SpendUpdateFailure for EntityCommitError<Write, Cache>
where
    Write: SpendUpdateFailure,
    Cache: StdError + Send + Sync + 'static,
{
    fn spend_failure(&self) -> SpendFailure {
        match self {
            Self::Write(error) => error.spend_failure(),
            // the table in flight never reached the database, or already committed and was marked, so nothing here may be dropped
            Self::TeamMemberKey(_) | Self::Cache { .. } => SpendFailure::Other,
        }
    }
}

pub struct EntitySpendCommit<'run, Writer, Cache> {
    pub writer: &'run Writer,
    pub policy: SpendRetryPolicy<'run>,
    pub cache: &'run Cache,
}

impl<Writer, Cache> EntitySpendCommit<'_, Writer, Cache>
where
    Writer: EntityTableWriter,
    Cache: ResetCache,
{
    // _commit_spend_updates_to_db: each table commits on its own in the source order, and every category, an empty one included, is marked before its cache entries are evicted.
    pub async fn commit_spend_updates<Progress>(
        &self,
        transactions: &SpendTableTransactions,
        mut on_table_committed: Progress,
    ) -> Result<(), EntityCommitError<Writer::Error, Cache::Failure>>
    where
        Progress: FnMut(SpendTable) + Send,
    {
        for table in SpendTable::COMMIT_ORDER {
            let increments = transactions
                .get(&table)
                .and_then(Option::as_ref)
                .filter(|rows| !rows.is_empty());
            tracing::debug!(
                table = table.name(),
                rows = increments.map_or(0, BTreeMap::len),
                "Spend transactions"
            );
            let evictions = match increments {
                Some(rows) => {
                    let keys =
                        cache_evictions(table, rows).map_err(EntityCommitError::TeamMemberKey)?;
                    self.commit_table(table, rows)
                        .await
                        .map_err(EntityCommitError::Write)?;
                    keys
                }
                None => Vec::new(),
            };
            on_table_committed(table);
            if !evictions.is_empty() {
                self.cache
                    .delete_user_api_keys(&evictions)
                    .await
                    .map_err(|source| EntityCommitError::Cache { table, source })?;
            }
        }
        Ok(())
    }

    // The retry loop of every table: n_retry_times + 1 attempts, each timed from its own start.
    async fn commit_table(
        &self,
        table: SpendTable,
        increments: &BTreeMap<String, Decimal>,
    ) -> Result<(), Writer::Error> {
        let mut attempt = 0_u32;
        loop {
            let started = self.policy.clock.instant();
            let Err(error) = self.writer.write_table(table, increments).await else {
                return Ok(());
            };
            match handle_spend_update_failure(&self.policy, &error, attempt, started).await {
                SpendRetry::Retry => attempt = attempt.saturating_add(1),
                SpendRetry::Stop => return Err(error),
            }
        }
    }
}

// The user_api_key_cache entries a committed table leaves stale: team memberships and projects, read from the raw keys before the write as the source does.
fn cache_evictions(
    table: SpendTable,
    increments: &BTreeMap<String, Decimal>,
) -> Result<Vec<String>, MemberKeyError> {
    let mut keys = Vec::new();
    match table {
        SpendTable::TeamMemberList => {
            for key in increments.keys() {
                let member = team_member_key(key)?;
                keys.push(format!(
                    "team_membership:{}:{}",
                    member.user_id, member.team_id
                ));
            }
        }
        SpendTable::ProjectList => {
            for project_id in increments.keys() {
                keys.push(format!("project_id:{project_id}"));
            }
        }
        SpendTable::UserList
        | SpendTable::EndUserList
        | SpendTable::KeyList
        | SpendTable::TeamList
        | SpendTable::OrgList
        | SpendTable::OrgMemberList
        | SpendTable::TagList
        | SpendTable::ModelAccessGroupList
        | SpendTable::AgentList => {}
    }
    Ok(keys)
}

mod tests;
