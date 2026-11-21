use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::cache::{UserCache, UserCacheError};
use betterlite_auth::auth_user_lookup::cadence::{Cadence, WallClock};
use betterlite_auth::auth_user_lookup::lookup::UserLookup;
use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, NoDefaultTeams, PortError};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_db::user_repository::projection::user_record_to_model;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_server::spend::visibility::error::VisibilityError;
use betterlite_server::spend::visibility::owners::OwnerPair;
use betterlite_server::spend::visibility::query::{OWNERS_SQL, PayloadQuery};
use betterlite_server::spend::visibility::service::SpendLogVisibility;
use betterlite_server::spend::visibility::store::{PayloadRow, VisibilityStore};
use betterlite_server::team_access::caller::TeamCaller;
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;
use tokio::sync::Mutex;
use tokio_util::task::TaskTracker;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    caller: CallerInput,
    lookup_id: String,
    users: Vec<UserInput>,
    teams: Vec<JsonData>,
    fail_teams: bool,
    owners: Vec<OwnerInput>,
    rows: Vec<RowInput>,
    payload: Option<JsonData>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallerInput {
    user_id: Option<String>,
    user_role: Option<UserRole>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserInput {
    user_id: String,
    teams: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnerInput {
    user: Option<String>,
    team_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RowInput {
    request_id: String,
    user: Option<String>,
    team_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct Read {
    read: &'static str,
    sql: Option<String>,
    binds: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct StepAnswer {
    reads: Vec<Read>,
    storage_key: Option<String>,
    outcome: &'static str,
    status: Option<u16>,
    detail: Option<Detail>,
}

#[derive(Debug, Serialize)]
struct Detail {
    error: String,
}

// the step's scripted spend-log and team reads, recorded in the order the checks make them
struct CaseStore {
    owners: Vec<OwnerPair>,
    rows: Vec<PayloadRow>,
    teams: Vec<LiteLLMTeamTable>,
    fail_teams: bool,
    reads: Mutex<Vec<Result<Read, serde_json::Error>>>,
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
        Ok(self
            .teams
            .iter()
            .find(|team| team.fields.base.team_id == team_id)
            .cloned())
    }

    async fn teams(&self, team_ids: &[String]) -> Result<Vec<LiteLLMTeamTable>, VisibilityError> {
        self.record("teams", None, serde_json::to_value([team_ids]))
            .await?;
        Ok(self
            .teams
            .iter()
            .filter(|team| team_ids.contains(&team.fields.base.team_id))
            .cloned()
            .collect())
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

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build()?;
        let mut answers = Vec::new();
        for step in &input.steps {
            answers.push(runtime.block_on(run_step(step))?);
        }
        Ok(serde_json::to_value(answers)?)
    }
}

async fn run_step(step: &Step) -> Result<StepAnswer, Box<HelperError>> {
    let store = Arc::new(case_store(step)?);
    let checks = SpendLogVisibility {
        store: Arc::<CaseStore>::clone(&store),
        users: Arc::new(user_lookup(&step.users)?),
    };
    let fields: BTreeMap<String, JsonData> =
        serde_json::from_value(serde_json::to_value(&step.caller)?)?;
    let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
    let caller = TeamCaller::try_from(&auth).or_value_error()?;
    let mut storage_key = None;
    let viewed = view(&checks, &caller, step, &mut storage_key).await;
    let reads = store
        .reads
        .lock()
        .await
        .drain(..)
        .collect::<Result<Vec<Read>, _>>()?;
    let (outcome, status, detail) = match viewed {
        Ok(served) => (served, None, None),
        Err(denied @ VisibilityError::Forbidden { .. }) => (
            "http",
            Some(403_u16),
            Some(Detail {
                error: denied.to_string(),
            }),
        ),
        Err(
            VisibilityError::Database(_)
            | VisibilityError::Team(_)
            | VisibilityError::TeamList(_)
            | VisibilityError::TeamRecord(_)
            | VisibilityError::Users(_),
        ) => ("error", None, None),
    };
    Ok(StepAnswer {
        reads,
        storage_key,
        outcome,
        status,
        detail,
    })
}

// the order ui_view_request_response_for_request_id runs the checks in, with one cold-storage logger
async fn view(
    checks: &SpendLogVisibility,
    caller: &TeamCaller,
    step: &Step,
    storage_key: &mut Option<String>,
) -> Result<&'static str, VisibilityError> {
    checks
        .assert_can_view_request_id(caller, &step.lookup_id)
        .await?;
    let resolved = checks.resolve_payload_row(caller, &step.lookup_id).await?;
    *storage_key = Some(resolved.storage_key);
    let Some(payload) = &step.payload else {
        return Ok(if resolved.row.is_some() {
            "row"
        } else {
            "none"
        });
    };
    checks
        .assert_owns_cold_payload(caller, payload, &step.lookup_id)
        .await?;
    Ok("payload")
}

fn case_store(step: &Step) -> Result<CaseStore, Box<HelperError>> {
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

fn user_lookup(users: &[UserInput]) -> Result<UserLookup, Box<HelperError>> {
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
