use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::cache::{UserCache, UserCacheError};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::auth_user_lookup::lookup::UserLookup;
use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, NoDefaultTeams, PortError};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_db::user_repository::projection::user_record_to_model;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::spend::visibility::error::VisibilityError;
use betterlite_server::spend::visibility::owners::OwnerPair;
use betterlite_server::spend::visibility::query::{OWNERS_SQL, PayloadQuery};
use betterlite_server::spend::visibility::store::{PayloadRow, VisibilityStore};
use serde::Serialize;
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;

use super::{Step, UserInput};

#[derive(Debug, Serialize)]
pub(super) struct Read {
    read: &'static str,
    sql: Option<String>,
    binds: serde_json::Value,
}

// the step's scripted spend-log and team reads, recorded in the order the checks make them
pub(super) struct CaseStore {
    owners: Vec<OwnerPair>,
    rows: Vec<PayloadRow>,
    teams: Vec<LiteLLMTeamTable>,
    fail_teams: bool,
    pub(super) reads: Mutex<Vec<Result<Read, serde_json::Error>>>,
}

impl CaseStore {
    // the SQL is compared with its whitespace collapsed, as LiteLLM indents its multi-line queries
    async fn record(
        &self,
        read: &'static str,
        statement: Option<&str>,
        binds: Result<serde_json::Value, serde_json::Error>,
    ) -> Result<(), VisibilityError> {
        let sql = statement.map(|text| text.split_whitespace().collect::<Vec<&str>>().join(" "));
        self.reads.lock().await.push(binds.map(|bound| Read {
            read,
            sql,
            binds: bound,
        }));
        if self.fail_teams && read != "query" {
            return Err(VisibilityError::Database(Box::new(DbError::NoDbConnected)));
        }
        Ok(())
    }

    // the stored team rows whose ids came back for the read
    fn matching_teams(&self, team_ids: &[&str]) -> Vec<LiteLLMTeamTable> {
        self.teams
            .iter()
            .filter(|team| team_ids.contains(&team.fields.base.team_id.as_str()))
            .cloned()
            .collect()
    }
}

#[async_trait]
impl VisibilityStore for CaseStore {
    async fn owners(&self, lookup_id: &str) -> Result<Vec<OwnerPair>, VisibilityError> {
        self.record("query", Some(OWNERS_SQL), serde_json::to_value([lookup_id]))
            .await?;
        Ok(self.owners.clone())
    }

    async fn payload_rows(
        &self,
        query: &PayloadQuery<'_>,
    ) -> Result<Vec<PayloadRow>, VisibilityError> {
        self.record("query", Some(query.sql), serde_json::to_value(query.binds))
            .await?;
        Ok(self.rows.clone())
    }

    async fn team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, VisibilityError> {
        self.record("team", None, serde_json::to_value([team_id]))
            .await?;
        Ok(self.matching_teams(&[team_id]).pop())
    }

    async fn teams(&self, team_ids: &[String]) -> Result<Vec<LiteLLMTeamTable>, VisibilityError> {
        let wanted: Vec<&str> = team_ids.iter().map(String::as_str).collect();
        self.record("teams", None, serde_json::to_value([team_ids]))
            .await?;
        Ok(self.matching_teams(&wanted))
    }
}

// the stored users a cache hit returns; a miss has no database behind it, so the lookup fails
struct CaseUsers {
    users: Vec<LiteLLMUserTable>,
}

#[async_trait]
impl UserCache for CaseUsers {
    async fn read(&self, key: &str) -> Result<Option<LiteLLMUserTable>, UserCacheError> {
        Ok(self
            .users
            .iter()
            .find(|stored| stored.user_id == key)
            .cloned())
    }

    async fn write(
        &self,
        _key: &str,
        _user: &LiteLLMUserTable,
        _ttl: f64,
    ) -> Result<(), UserCacheError> {
        Ok(())
    }

    fn management_ttl(&self) -> f64 {
        60.0_f64
    }
}

struct NoReset;

impl BudgetResetCalculator for NoReset {
    fn reset_time(&self, _budget_duration: &str) -> Result<Timestamp, PortError> {
        Err(PortError::new("visibility never computes a budget reset"))
    }
}

pub(super) fn case_store(step: &Step) -> Result<CaseStore, Box<HelperError>> {
    let teams = step
        .teams
        .iter()
        .map(|team| LiteLLMTeamTable::try_from(team.clone()))
        .collect::<Result<Vec<_>, _>>()
        .or_value_error()?;
    let owners = step
        .owners
        .iter()
        .map(|owner| OwnerPair {
            user: owner.user.clone(),
            team_id: owner.team_id.clone(),
        })
        .collect();
    let rows = step
        .rows
        .iter()
        .map(|row| PayloadRow {
            request_id: row.request_id.clone(),
            messages: None,
            response: None,
            proxy_server_request: None,
            metadata: None,
            user: row.user.clone(),
            team_id: row.team_id.clone(),
        })
        .collect();
    Ok(CaseStore {
        owners,
        rows,
        teams,
        fail_teams: step.fail_teams,
        reads: Mutex::new(Vec::new()),
    })
}

pub(super) fn user_lookup(users: &[UserInput]) -> Result<UserLookup, Box<HelperError>> {
    let mut stored = Vec::with_capacity(users.len());
    for user in users {
        let record: JsonData = serde_json::from_value(serde_json::to_value(user)?)?;
        if let Some(found) = user_record_to_model(Some(&record)).or_value_error()? {
            stored.push(found);
        }
    }
    Ok(UserLookup {
        cache: Arc::new(CaseUsers { users: stored }),
        store: None,
        cadence: Arc::new(Cadence::new(100, Arc::new(WallClock))),
        db_cache_expiry: 5,
        budget_reset: Arc::new(NoReset),
        enrollment: Arc::new(NoDefaultTeams),
        tasks: TaskTracker::new(),
        defaults: None,
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).or_value_error()?),
    })
}
