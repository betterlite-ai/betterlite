use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;
use async_trait::async_trait;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::safe_json::MAX_JSON_BYTES;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_secret_managers::secret_helpers::SecretManagerReadState;
use betterlite_secret_managers::secret_types::SecretAccessMode;
use betterlite_server::judge_targets::call::JudgeCall;
use betterlite_server::judge_targets::error::JudgeDispatchError;
use betterlite_server::judge_targets::sdk::JudgeSdk;
use betterlite_server::model_visibility::testing::{Row, Snapshot};
use betterlite_server::shadow_eval_validation::access::{
    require_admin_viewer, require_admin_writer,
};
use betterlite_server::shadow_eval_validation::error::{AccessError, ShadowEvalRequestError};
use betterlite_server::shadow_eval_validation::request::{
    ShadowEvalDirection, StartShadowEvalRequest, parse_shadow_eval_request,
};
use betterlite_server::shadow_eval_validation::router_view::{
    AnthropicCredentials, ShadowEvalRouter,
};
use betterlite_server::shadow_eval_validation::start::{
    StartContext, TargetKey, TargetRecords, validate_start,
};
use rust_decimal::prelude::ToPrimitive as _;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Input {
    pub router: Vec<Deployment>,
    pub rows: Rows,
    pub steps: Vec<Step>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deployment {
    pub model_name: String,
    pub litellm_params: JsonObject,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRow {
    pub token: String,
    #[serde(default)]
    pub team_id: Option<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rows {
    pub keys: Vec<KeyRow>,
    pub teams: Vec<String>,
    pub users: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Parse(Body),
    Access(Role),
    ValidateStart(Body),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Body {
    pub text: String,
    #[serde(default)]
    pub pad_to: Option<usize>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Absent,
    ProxyAdmin,
    ProxyAdminViewer,
    InternalUser,
    Customer,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub steps: Vec<StepOutput>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", content = "result", rename_all = "snake_case")]
pub enum StepOutput {
    Parse(ParseResult),
    Access { writer: Verdict, viewer: Verdict },
    ValidateStart(StartResult),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ParseResult {
    Accepted { request: RequestView },
    Rejected,
    OverLimit { bytes: usize, limit: usize },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Allowed,
    Denied,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum StartResult {
    Validated,
    Rejected { status: u16 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestView {
    pub api_key_ids: Vec<String>,
    pub team_ids: Vec<String>,
    pub user_ids: Vec<String>,
    pub models: Vec<String>,
    pub router_names: Vec<String>,
    pub direction: ShadowEvalDirection,
    pub baseline_model: Option<String>,
    pub shadow_percentage: String,
    pub judge_model: String,
    pub duration_days: u32,
    pub max_budget: String,
}
struct NoSdk;
#[async_trait]
impl JudgeSdk for NoSdk {
    async fn acompletion(&self, _call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        Err(JudgeDispatchError {
            message: "start-time validation dispatches no judge call".to_owned(),
        })
    }
}
fn invalid<E>(error: E) -> OperationError
where
    E: Display,
{
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}
fn auth(role: Role) -> Result<UserAPIKeyAuth, OperationError> {
    let name = match role {
        Role::Absent => None,
        Role::ProxyAdmin => Some("proxy_admin"),
        Role::ProxyAdminViewer => Some("proxy_admin_viewer"),
        Role::InternalUser => Some("internal_user"),
        Role::Customer => Some("customer"),
    };
    let mut fields = BTreeMap::new();
    if let Some(declared) = name {
        fields.insert(
            "user_role".to_owned(),
            JsonData::String(declared.to_owned()),
        );
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).map_err(invalid)
}
fn verdict<T>(result: Result<T, AccessError>) -> Verdict {
    match result {
        Ok(_allowed) => Verdict::Allowed,
        Err(_denied) => Verdict::Denied,
    }
}
fn body_bytes(body: &Body) -> Result<Vec<u8>, OperationError> {
    let mut bytes = body.text.clone().into_bytes();
    let limit = MAX_JSON_BYTES.saturating_add(1);
    let total = body.pad_to.unwrap_or(bytes.len());
    if total < bytes.len() || total > limit {
        return Err(invalid(format!("pad_to must be {}..={limit}", bytes.len())));
    }
    bytes.resize(total, b' ');
    Ok(bytes)
}
fn view(request: &StartShadowEvalRequest) -> RequestView {
    RequestView {
        api_key_ids: request.api_key_ids.clone(),
        team_ids: request.team_ids.clone(),
        user_ids: request.user_ids.clone(),
        models: request.models.clone(),
        router_names: request.router_names.clone(),
        direction: request.direction,
        baseline_model: request.baseline_model.clone(),
        shadow_percentage: format!("{:.2}", request.shadow_percentage),
        judge_model: request.judge_model.clone(),
        duration_days: request.duration_days,
        max_budget: format!("{:.15}", request.max_budget.to_f64().unwrap_or_default()),
    }
}
fn parse_result(body: &Body) -> Result<ParseResult, OperationError> {
    match parse_shadow_eval_request(&body_bytes(body)?) {
        Ok(request) => Ok(ParseResult::Accepted {
            request: view(&request),
        }),
        Err(ShadowEvalRequestError::Bounded(PrivateJsonError::TooLarge(bytes, limit))) => {
            Ok(ParseResult::OverLimit { bytes, limit })
        }
        Err(ShadowEvalRequestError::Bounded(..))
        | Err(ShadowEvalRequestError::Decode(..))
        | Err(ShadowEvalRequestError::Field { .. })
        | Err(ShadowEvalRequestError::NotAnObject)
        | Err(ShadowEvalRequestError::Validation { .. }) => Ok(ParseResult::Rejected),
    }
}
fn start_result(body: &Body, context: &StartContext<'_>) -> Result<StartResult, OperationError> {
    let request = parse_shadow_eval_request(&body_bytes(body)?).map_err(invalid)?;
    match validate_start(&request, context) {
        Ok(()) => Ok(StartResult::Validated),
        Err(error) => Ok(StartResult::Rejected {
            status: error.status_code(),
        }),
    }
}
fn step_output(step: &Step, context: &StartContext<'_>) -> Result<StepOutput, OperationError> {
    match step {
        Step::Parse(body) => Ok(StepOutput::Parse(parse_result(body)?)),
        Step::Access(role) => {
            let caller = auth(*role)?;
            Ok(StepOutput::Access {
                writer: verdict(require_admin_writer(&caller, "start a shadow eval")),
                viewer: verdict(require_admin_viewer(&caller, "view shadow evals")),
            })
        }
        Step::ValidateStart(body) => Ok(StepOutput::ValidateStart(start_result(body, context)?)),
    }
}
pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: catalog.as_ref(),
        known_model_names: &known,
        custom_providers: &custom,
        environment: &environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let cache = ProviderCache::default();
    let sdk = NoSdk;
    let info = Json::Object(JsonObject::new());
    let rows: Vec<Row<'_>> = input
        .router
        .iter()
        .map(|deployment| Row {
            model_name: &deployment.model_name,
            litellm_params: &deployment.litellm_params,
            model_info: &info,
        })
        .collect();
    let fixture = Snapshot::build(&rows, &JsonObject::new()).map_err(invalid)?;
    let provider = |_model: &str| -> Option<String> { None };
    let settings = AnthropicSettings::default();
    let keys: Vec<TargetKey<'_>> = input
        .rows
        .keys
        .iter()
        .map(|row| TargetKey {
            token: &row.token,
            team_id: row.team_id.as_deref(),
        })
        .collect();
    let teams: Vec<&str> = input.rows.teams.iter().map(String::as_str).collect();
    let users: Vec<&str> = input.rows.users.iter().map(String::as_str).collect();
    let context = StartContext {
        router: Some(ShadowEvalRouter {
            catalog: &fixture.catalog,
            model_group_alias: &fixture.aliases,
            routing_groups: &fixture.groups,
            inferred_provider: &provider,
        }),
        sdk: &sdk,
        cache: &cache,
        resolution: &resolution,
        credentials: AnthropicCredentials {
            anthropic_key: None,
            api_key: None,
            settings: &settings,
            secret_state: SecretManagerReadState {
                client_exists: false,
                settings_exist: false,
                access_mode: SecretAccessMode::ReadOnly,
                hosted_keys: None,
            },
        },
        records: TargetRecords {
            keys: &keys,
            teams: &teams,
            users: &users,
        },
    };
    let steps = input
        .steps
        .iter()
        .map(|step| step_output(step, &context))
        .collect::<Result<Vec<StepOutput>, OperationError>>()?;
    Ok(ProbeOutput::ShadowEvalValidation(Output { steps }))
}
