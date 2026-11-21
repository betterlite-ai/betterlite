use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_auth::management_routes::key_routes::{KEY_ENUM, KeyManagementRoute};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_auth::user_roles::UserRole;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::key_access::Settings;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::Payload;
use betterlite_db::team_repository::repository::TeamQueryError;
use betterlite_db::team_rows::team::TeamBase;
use betterlite_logging::callback_config::FeatureFlags;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::key_access::common::common_key_access_checks;
use betterlite_server::key_access::custom_key::check_custom_key_allowed;
use betterlite_server::key_access::error::{DetailShape, KeyAccessError};
use betterlite_server::key_access::generation::key_generation_check;
use betterlite_server::key_access::logging_metadata::check_key_logging_config;
use betterlite_server::key_access::service_account::{
    TeamDirectory, validate_service_account_team,
};
use betterlite_server::key_fields::decode::TypedValue;
use betterlite_server::key_fields::request::GenerateKeyRequest;
use betterlite_server::key_fields::values::LiteLLMObjectPermissionBase;
use betterlite_server::team_access::caller::TeamCaller;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder, Runtime};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

// one call of an owned function; each step names every field so the Python TypedDict needs no defaults
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Step {
    function: Function,
    config: JsonData,
    caller: CallerInput,
    team: Option<TeamBase<String>>,
    data: TypedValue,
    route: String,
    user_id: Option<String>,
    custom_key: Option<String>,
    metadata: Option<Payload>,
    team_id: Option<String>,
    database: bool,
    teams: Vec<String>,
    object_permission: Option<LiteLLMObjectPermissionBase>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    CustomKey,
    Generation,
    Common,
    LoggingMetadata,
    ServiceTeam,
    ObjectPermission,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CallerInput {
    user_id: Option<String>,
    user_role: Option<UserRole>,
    team_id: Option<String>,
    metadata: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Serialize)]
struct StepResult {
    value: Option<LiteLLMObjectPermissionBase>,
    error: Option<ErrorOut>,
    warnings: Vec<String>,
    lookups: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ErrorOut {
    status: Option<u16>,
    detail: Option<DetailShape>,
    error_type: Option<String>,
    param: Option<String>,
    code: Option<String>,
}

// the team table a service-account step reads, recording every team id it is asked about
struct CaseTeams {
    known: Vec<String>,
    asked: Mutex<Vec<String>>,
}

#[async_trait]
impl TeamDirectory for CaseTeams {
    async fn team_exists(&self, team_id: &str) -> Result<bool, TeamQueryError> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(team_id.to_owned());
        Ok(self.known.iter().any(|known| known == team_id))
    }
}

#[derive(Clone, Default)]
struct WarningLines(Arc<Mutex<Vec<String>>>);

struct MessageField(Option<String>);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for WarningLines {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = MessageField(None);
        event.record(&mut message);
        if let Some(text) = message
            .0
            .filter(|_| *event.metadata().level() == Level::WARN)
        {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(text);
        }
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build()?;
        let mut results = Vec::new();
        for step in &input.steps {
            results.push(run_step(&runtime, step)?);
        }
        Ok(serde_json::to_value(results)?)
    }
}

fn run_step(runtime: &Runtime, step: &Step) -> Result<StepResult, Box<HelperError>> {
    let source = serde_json::to_string(&step.config)?;
    let lookup = BTreeMap::<String, String>::new();
    let settings = ConfigLoader { lookup }.load(&source).or_config_error()?;
    let caller = step.team_caller()?;
    let teams = CaseTeams {
        known: step.teams.clone(),
        asked: Mutex::new(Vec::new()),
    };
    let call = Call {
        step,
        route: step.key_route()?,
        caller: &caller,
        settings: &settings.sections.key_access,
        teams: &teams,
    };
    let (outcome, warnings) = call.captured(runtime);
    // _object_permission_to_dict is the exclude_unset dump B770's nullable fields serialize to
    let permission = matches!(step.function, Function::ObjectPermission)
        .then(|| step.object_permission.clone())
        .flatten();
    let (value, error) = match outcome {
        Ok(()) => (permission, None),
        Err(failure) => (None, Some(error_out(&failure))),
    };
    let lookups = teams
        .asked
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    Ok(StepResult {
        value,
        error,
        warnings,
        lookups,
    })
}

impl Step {
    // the case's caller becomes a real key first, so the production conversion runs
    fn team_caller(&self) -> Result<TeamCaller, Box<HelperError>> {
        let fields: BTreeMap<String, JsonData> =
            serde_json::from_value(serde_json::to_value(&self.caller)?)?;
        let auth = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
        TeamCaller::try_from(&auth).or_value_error()
    }

    fn key_route(&self) -> Result<KeyManagementRoute, Box<HelperError>> {
        KEY_ENUM
            .iter()
            .copied()
            .find(|known| known.value() == self.route)
            .ok_or_else(|| format!("{} is not a key management route", self.route))
            .or_value_error()
    }
}

// the status, detail shape and ProxyException fields LiteLLM raises; the message is betterlite's own
fn error_out(failure: &KeyAccessError) -> ErrorOut {
    let exception = failure.proxy_exception();
    ErrorOut {
        status: failure.status(),
        detail: failure.detail(),
        error_type: exception.as_ref().map(|found| found.error_type.clone()),
        param: exception.as_ref().and_then(|found| found.param.clone()),
        code: exception.map(|found| found.code),
    }
}

struct Call<'step> {
    step: &'step Step,
    route: KeyManagementRoute,
    caller: &'step TeamCaller,
    settings: &'step Settings,
    teams: &'step CaseTeams,
}

impl Call<'_> {
    fn captured(&self, runtime: &Runtime) -> (Result<(), KeyAccessError>, Vec<String>) {
        let lines = WarningLines::default();
        let subscriber = tracing_subscriber::registry().with(lines.clone());
        let outcome = with_default(subscriber, || runtime.block_on(self.answer()));
        let warnings = lines
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        (outcome, warnings)
    }

    async fn answer(&self) -> Result<(), KeyAccessError> {
        let step = self.step;
        match step.function {
            Function::CustomKey => {
                let secret = step.custom_key.clone().map(SecretString::from);
                check_custom_key_allowed(secret.as_ref(), self.settings)
            }
            Function::Generation => {
                let data = GenerateKeyRequest::read(&step.data)?;
                let team = step.team.as_ref();
                let generation = &self.settings.key_generation;
                key_generation_check(self.caller, team, &data, self.route, generation)
            }
            Function::Common => {
                let data = GenerateKeyRequest::read(&step.data)?;
                common_key_access_checks(self.caller, &data.base, step.user_id.as_deref())
            }
            Function::LoggingMetadata => {
                let flags = FeatureFlags {
                    otel_v2_enabled: false,
                };
                check_key_logging_config(step.metadata.as_ref(), flags)
            }
            Function::ServiceTeam => {
                let directory: &dyn TeamDirectory = self.teams;
                let teams = step.database.then_some(directory);
                validate_service_account_team(step.team_id.as_deref(), teams).await
            }
            Function::ObjectPermission => Ok(()),
        }
    }
}
