use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_model_patterns::pattern::UnresolvedProviderInference;
use betterlite_auth::auth_model_patterns::user_model::can_user_call_model;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::entity_model_access::error::EntityModelAccessError;
use betterlite_auth::entity_model_access::groups::{ResourceKind, authorized_resources};
use betterlite_auth::entity_model_access::key::can_key_call_model;
use betterlite_auth::entity_model_access::org::can_org_access_model;
use betterlite_auth::entity_model_access::project::can_project_access_model;
use betterlite_auth::entity_model_access::resolved::{ResolvedDeps, can_key_call_resolved_model};
use betterlite_auth::entity_model_access::team::can_team_access_model;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_db::resource_rows::project::ProjectTable;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use fixture::{Fixture, admission, project_lookup, team_lookup};

#[path = "entity_model_access/fixtures/mod.rs"]
mod fixture;

static UNRESOLVED_PROVIDER: UnresolvedProviderInference = UnresolvedProviderInference;
static NO_ALIASES: BTreeMap<String, String> = BTreeMap::new();
// litellm.constants' DEFAULT_MAX_RECURSE_DEPTH default
const MAX_RECURSE_DEPTH: i64 = 100;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

// one call of the owned behavior; `cache` seeds the proxy cache the calls read through
#[derive(Debug, Deserialize)]
pub(crate) struct Step {
    function: Function,
    #[serde(default)]
    model: String,
    #[serde(default)]
    database: bool,
    #[serde(default)]
    token: Option<Value>,
    #[serde(default)]
    team: Option<Value>,
    #[serde(default)]
    org: Option<Value>,
    #[serde(default)]
    project: Option<Value>,
    #[serde(default)]
    user: Option<Value>,
    #[serde(default)]
    cache: BTreeMap<String, Value>,
    #[serde(default)]
    aliases: BTreeMap<String, String>,
    #[serde(default)]
    alias_targets: BTreeMap<String, String>,
    #[serde(default)]
    resource_field: Option<Field>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Key,
    Team,
    Org,
    Project,
    Resolved,
    Resources,
    User,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Field {
    ModelNames,
    McpServerIds,
    AgentIds,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let steps = runtime.block_on(step_values(&input.steps))?;
        Ok(json!(steps))
    }
}

async fn step_values(steps: &[Step]) -> Result<Vec<Value>, Box<HelperError>> {
    let mut values = Vec::with_capacity(steps.len());
    for step in steps {
        values.push(step_value(step).await?);
    }
    Ok(values)
}

async fn step_value(step: &Step) -> Result<Value, Box<HelperError>> {
    let fixture = Arc::new(Fixture::build(step)?);
    let lookup = step.database.then(|| team_lookup(&fixture));
    let model = RequestedModels::One(step.model.clone());
    let team: Option<LiteLLMTeamTable> = row(step.team.as_ref())?;
    let aliases = &step.aliases;
    let targets = &step.alias_targets;
    match step.function {
        Function::Key => {
            let token = token(step)?;
            let outcome = can_key_call_model(&model, &token, &context(), lookup.as_ref()).await;
            Ok(decision_value(outcome))
        }
        Function::Team => {
            let outcome = can_team_access_model(
                &model,
                team.as_ref(),
                aliases,
                targets,
                &context(),
                lookup.as_ref(),
            )
            .await;
            Ok(decision_value(outcome))
        }
        Function::Org => {
            let org: Option<OrganizationTable> = row(step.org.as_ref())?;
            let outcome = can_org_access_model(&model, org.as_ref(), aliases, &context());
            Ok(decision_value(outcome))
        }
        Function::Project => {
            let project: Option<ProjectTable> = row(step.project.as_ref())?;
            let outcome = can_project_access_model(&model, project.as_ref(), targets, &context());
            Ok(decision_value(outcome))
        }
        Function::Resolved => {
            let token = token(step)?;
            let context = context();
            let projects = step.database.then(|| project_lookup(&fixture));
            let member = admission(&fixture, &context);
            let deps = ResolvedDeps {
                teams: lookup.as_ref(),
                projects: projects.as_ref(),
                admission: Some(&member),
                now: 0.0_f64,
            };
            let outcome = can_key_call_resolved_model(&model, &token, &context, &deps).await;
            Ok(decision_value(outcome))
        }
        Function::Resources => {
            let token = token(step)?;
            let mut resources =
                authorized_resources(Some(&token), team.as_ref(), lookup.as_ref(), kind(step))
                    .await;
            // Python hands the set back in hash order, so both sides answer sorted
            resources.sort_unstable();
            Ok(json!({"allowed": true, "error": Value::Null, "resources": resources}))
        }
        Function::User => {
            let user: Option<LiteLLMUserTable> = row(step.user.as_ref())?;
            let decision = can_user_call_model(&model, user.as_ref(), targets, &context());
            Ok(decision_value(EntityModelAccessError::gate(decision)))
        }
    }
}

fn context() -> ModelAccessContext<'static> {
    ModelAccessContext {
        view: None,
        global_model_aliases: &NO_ALIASES,
        compaction: None,
        provider: &UNRESOLVED_PROVIDER,
        max_recurse_depth: MAX_RECURSE_DEPTH,
    }
}

fn kind(step: &Step) -> ResourceKind {
    match step.resource_field.unwrap_or(Field::ModelNames) {
        Field::ModelNames => ResourceKind::ModelNames,
        Field::McpServerIds => ResourceKind::McpServerIds,
        Field::AgentIds => ResourceKind::AgentIds,
    }
}

fn row<T: DeserializeOwned>(value: Option<&Value>) -> Result<Option<T>, serde_json::Error> {
    value
        .map(|row| serde_json::from_value(row.clone()))
        .transpose()
}

fn token(step: &Step) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let value = step.token.clone().unwrap_or(Value::Null);
    let fields: BTreeMap<String, JsonData> = serde_json::from_value(value)?;
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()
}

fn decision_value(outcome: Result<(), EntityModelAccessError>) -> Value {
    match outcome {
        Ok(()) => json!({"allowed": true, "error": Value::Null, "resources": Value::Null}),
        Err(error) => {
            json!({"allowed": false, "error": error_value(&error), "resources": Value::Null})
        }
    }
}

fn error_value(error: &EntityModelAccessError) -> Value {
    let Some(denial) = error.denial() else {
        return json!({
            "class": class_name(error),
            "status": Value::Null,
            "type": Value::Null,
            "param": Value::Null,
            "code": Value::Null,
            "message": error.to_string(),
        });
    };
    let exception = denial.as_proxy_exception();
    json!({
        "class": "ModelAccessDeniedProxyException",
        "status": exception.code.parse::<u16>().ok(),
        "type": exception.error_type,
        "param": exception.param,
        "code": exception.code,
        "message": exception.message,
    })
}

fn class_name(error: &EntityModelAccessError) -> &'static str {
    match error {
        EntityModelAccessError::Denied(_) => "ModelAccessDeniedProxyException",
        EntityModelAccessError::RecurseDepth { .. } => "Exception",
        EntityModelAccessError::Member(_) => "TeamBudgetError",
        EntityModelAccessError::Project(_) => "OrgLookupError",
        EntityModelAccessError::TeamRow(_) => "TeamRowError",
    }
}
