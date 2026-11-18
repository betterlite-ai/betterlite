use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::combined_key_read::error::{KeyReadError, ReconnectFailure};
use betterlite_db::combined_key_read::grace::{DeprecatedKeyCache, DeprecatedToken};
use betterlite_db::combined_key_read::ports::NoSpanResolver;
use betterlite_db::combined_key_read::reader::{KeyReader, ReadServices};
use betterlite_db::combined_key_read::request::{CombinedKeyRequest, KeyQueryType};
use betterlite_db::combined_key_read::view::CombinedKeyView;
use betterlite_db::database_faults::{DbError, ErrorType};
use betterlite_db::identity_rows::key::Expiry;
use betterlite_db::team_rows::member::Member;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;
use serde::Serialize;
use tokio::runtime::Builder;
use tokio_util::task::TaskTracker;

use super::fakes::{
    CountingDiagnostics, CountingHook, CountingPause, DeprecatedStep, FullJitter,
    RecordedReconnect, RecordingSink, RowStep, ScriptedDatabase, ScriptedReconnect, StepClock,
};
use super::{DeprecatedFields, Fault, Input, QueryType, ReconnectStep, Step};

pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    runtime.block_on(execute(input))
}

async fn execute(input: &Input) -> HelperResult {
    let state = ServiceState::new(input)?;
    let outcome = state.read(input).await;
    let report = Report::build(&state, outcome);
    serde_json::to_value(&report).or_value_error()
}
struct ServiceState {
    database: ScriptedDatabase,
    reconnect: ScriptedReconnect,
    sink: Arc<RecordingSink>,
    clock: StepClock,
    jitter: FullJitter,
    pause: CountingPause,
    diagnostics: CountingDiagnostics,
    hook: CountingHook,
    tasks: TaskTracker,
    cache: Arc<Mutex<DeprecatedKeyCache>>,
}
impl ServiceState {
    fn new(input: &Input) -> Result<Self, Box<HelperError>> {
        let wall = DateTime::parse_from_rfc3339(&input.now)
            .or_value_error()?
            .with_timezone(&Utc);
        Ok(Self {
            database: ScriptedDatabase::new(
                row_steps(&input.rows)?,
                deprecated_steps(&input.deprecated)?,
            ),
            reconnect: ScriptedReconnect::new(reconnect_steps(&input.reconnects)),
            sink: Arc::new(RecordingSink::default()),
            clock: StepClock::new(Duration::from_millis(input.step_millis), wall),
            jitter: FullJitter,
            pause: CountingPause::default(),
            diagnostics: CountingDiagnostics::default(),
            hook: CountingHook::default(),
            tasks: TaskTracker::new(),
            cache: Arc::new(Mutex::new(DeprecatedKeyCache::default())),
        })
    }
    async fn read(&self, input: &Input) -> Result<Option<CombinedKeyView>, KeyReadError> {
        let query_type = match input.query_type {
            QueryType::FindUnique => KeyQueryType::FindUnique,
            QueryType::FindAll => KeyQueryType::FindAll,
        };
        let request = CombinedKeyRequest {
            token: input.token.as_deref(),
            query_type,
            check_deprecated: input.check_deprecated,
            use_writer: input.use_writer,
            parent_otel_span: None,
        };
        let reader = KeyReader {
            database: &self.database,
            reconnect: &self.reconnect,
            sink: Arc::clone(&self.sink),
            services: ReadServices {
                clock: &self.clock,
                jitter: &self.jitter,
                pause: &self.pause,
                diagnostics: &self.diagnostics,
                hook: &self.hook,
                resolver: &NoSpanResolver,
                tasks: &self.tasks,
            },
            cache: Arc::clone(&self.cache),
        };
        let outcome = reader.read(&request).await;
        self.tasks.close();
        self.tasks.wait().await;
        outcome
    }
}
fn row_steps(steps: &[Step<BTreeMap<String, JsonData>>]) -> Result<Vec<RowStep>, Box<HelperError>> {
    steps
        .iter()
        .map(|step| match step {
            Step::Failure(failure) => Ok(Err(database_fault(&failure.error)?)),
            Step::Data(data) => Ok(Ok(Some(JsonData::Object(data.data.clone())))),
            Step::Missing => Ok(Ok(None)),
        })
        .collect()
}
fn deprecated_steps(
    steps: &[Step<DeprecatedFields>],
) -> Result<Vec<DeprecatedStep>, Box<HelperError>> {
    steps
        .iter()
        .map(|step| match step {
            Step::Failure(failure) => Ok(Err(database_fault(&failure.error)?)),
            Step::Data(data) => {
                let revoke_at = DateTime::from_timestamp(data.data.revoke_at_seconds, 0)
                    .ok_or_else(|| {
                        value_error(String::from("revoke_at_seconds is out of range"))
                    })?;
                Ok(Ok(Some(DeprecatedToken {
                    active_token_id: data.data.active_token_id.clone(),
                    revoke_at,
                })))
            }
            Step::Missing => Ok(Ok(None)),
        })
        .collect()
}
fn reconnect_steps(steps: &[ReconnectStep]) -> Vec<Result<bool, ReconnectFailure>> {
    steps
        .iter()
        .map(|step| match step {
            ReconnectStep::Outcome(outcome) => Ok(*outcome),
            ReconnectStep::Missing => Ok(true),
        })
        .collect()
}
fn database_fault(fault: &Fault) -> Result<DbError, Box<HelperError>> {
    let error_type = match fault.error_type.as_str() {
        "raw_query" => ErrorType::RawQuery,
        "data" => ErrorType::Data,
        "connect" => ErrorType::Connect,
        "other" => ErrorType::Other,
        unknown => return Err(value_error(format!("unknown error type {unknown}"))),
    };
    Ok(DbError {
        error_type,
        message: fault.message.clone(),
        code: None,
        meta_code: None,
        traceback_modules: Vec::new(),
        cause: None,
    })
}
fn value_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: Some(None),
        param: Some(None),
        code: Some(None),
        message,
    })
}
#[derive(Serialize)]
struct Report {
    result: &'static str,
    view: Option<ViewReport>,
    error: Option<ErrorReport>,
    row_queries: u32,
    deprecated_queries: u32,
    reconnects: Vec<ReconnectReport>,
    metric_success: u32,
    metric_failure: u32,
    hooks: u32,
    backoffs: u32,
    cached_plan_warnings: u32,
    deprecated_used: bool,
}
impl Report {
    fn build(state: &ServiceState, outcome: Result<Option<CombinedKeyView>, KeyReadError>) -> Self {
        let (result, view, error) = match outcome {
            Ok(Some(found)) => ("found", Some(ViewReport::from_view(&found)), None),
            Ok(None) => ("none", None, None),
            Err(failure) => (
                failure.class(),
                None,
                Some(ErrorReport {
                    kind: failure.class(),
                    status: failure.status_code(),
                }),
            ),
        };
        Self {
            result,
            view,
            error,
            row_queries: state.database.row_queries(),
            deprecated_queries: state.database.deprecated_queries(),
            reconnects: state
                .reconnect
                .requests()
                .iter()
                .map(ReconnectReport::from_recorded)
                .collect(),
            metric_success: state.sink.successes(),
            metric_failure: state.sink.failures(),
            hooks: state.hook.hooks(),
            backoffs: state.diagnostics.backoffs(),
            cached_plan_warnings: state.diagnostics.cached_plan_warnings(),
            deprecated_used: state.diagnostics.deprecated_used(),
        }
    }
}
#[derive(Serialize)]
struct ViewReport {
    token: Option<String>,
    team_alias: Option<String>,
    team_models: Vec<JsonData>,
    team_blocked: bool,
    team_member: Option<Member>,
    org_id: Option<String>,
    organization_alias: Option<String>,
    project_alias: Option<String>,
    team_tpm_limit: Option<i64>,
    team_member_tpm_limit: Option<i64>,
    expires: Option<String>,
}
impl ViewReport {
    fn from_view(view: &CombinedKeyView) -> Self {
        Self {
            token: view
                .token
                .token
                .as_ref()
                .map(|token| token.expose_secret().to_owned()),
            team_alias: view.team_alias.clone(),
            team_models: view.team_models.clone(),
            team_blocked: view.team_blocked,
            team_member: view.team_member.clone(),
            org_id: view.token.org_id.clone(),
            organization_alias: view.organization_alias.clone(),
            project_alias: view.project_alias.clone(),
            team_tpm_limit: view.team_tpm_limit,
            team_member_tpm_limit: view.team_member_tpm_limit,
            expires: view.token.expires.as_ref().map(expiry_text),
        }
    }
}
fn expiry_text(expiry: &Expiry) -> String {
    match expiry {
        Expiry::Text(text) => text.clone(),
        Expiry::DateTime(stamp) => stamp.to_rfc3339(),
    }
}
#[derive(Serialize)]
struct ErrorReport {
    kind: &'static str,
    status: Option<u16>,
}
#[derive(Serialize)]
struct ReconnectReport {
    reason: String,
    force_recreate: bool,
    timeout_seconds: Option<f64>,
    lock_timeout_seconds: Option<f64>,
    #[serde(flatten)]
    stale: StaleReport,
}
#[derive(Serialize)]
struct StaleReport {
    stale_present: bool,
    stale_live: bool,
}

impl ReconnectReport {
    fn from_recorded(recorded: &RecordedReconnect) -> Self {
        Self {
            reason: recorded.reason.clone(),
            force_recreate: recorded.force_recreate,
            timeout_seconds: recorded.timeout.map(|timeout| timeout.as_secs_f64()),
            lock_timeout_seconds: recorded.lock_timeout.map(|timeout| timeout.as_secs_f64()),
            stale: StaleReport {
                stale_present: recorded.stale.is_some(),
                stale_live: recorded.stale.unwrap_or(false),
            },
        }
    }
}
