mod effects;
mod script;

use std::collections::BTreeMap;

use betterlite_auth::auth_model_grants::access_source::AgentAccessSource;
use betterlite_auth::auth_model_grants::caller::check_agent_caller_model_access;
use betterlite_auth::auth_model_grants::ceiling::check_agent_access_group_model_access;
use betterlite_auth::auth_model_grants::error::AgentModelGrantsError;
use betterlite_auth::auth_model_grants::fallback::RouterFallbackAccessCheck;
use betterlite_auth::auth_model_grants::source_adapter::user_api_key_auth_from_request;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::entity_model_access::resolved::ResolvedDeps;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::effects::Effects;
use self::script::{Script, admission, context};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Step {
    function: Function,
    model: String,
    #[serde(default)]
    token: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    request: BTreeMap<String, JsonData>,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    managed: Option<Value>,
    #[serde(default)]
    managed_ceilings: Vec<Vec<String>>,
    #[serde(default)]
    unmanaged: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    Agent,
    Fallback,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let mut answers = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            let value = runtime.block_on(step_value(step))?;
            answers.push(value);
        }
        Ok(json!(answers))
    }
}

async fn step_value(step: &Step) -> HelperResult {
    let effects = Effects::default();
    match step.function {
        Function::Agent => agent_value(step, &effects).await,
        Function::Fallback => fallback_value(step).await,
    }
}

// both production checks in the stage's order, with the scripted source standing in for B8646's registry
async fn agent_value(step: &Step, effects: &Effects) -> HelperResult {
    let token = token_of(step)?;
    let script = Script {
        step,
        effects: effects.clone(),
    };
    let source: &dyn AgentAccessSource = &script;
    let model = RequestedModels::One(step.model.clone());
    let access = context();
    let ceiling =
        check_agent_access_group_model_access(&model, token.as_ref(), Some(source), &access).await;
    let outcome = match ceiling {
        Ok(()) => {
            let member = admission(&access);
            check_agent_caller_model_access(&model, token.as_ref(), Some(source), &member, &access)
                .await
        }
        Err(error) => Err(error),
    };
    Ok(step_result(outcome, effects.entries()))
}

// RouterFallbackAccessCheck.__call__ with the source adapter's token, exactly as the router passes it
async fn fallback_value(step: &Step) -> HelperResult {
    let token = user_api_key_auth_from_request(&step.request);
    let access = context();
    let deps = ResolvedDeps {
        teams: None,
        projects: None,
        admission: None,
        now: 0.0_f64,
    };
    let check = RouterFallbackAccessCheck {
        is_enforced: step.enabled,
        token: token.as_ref(),
        context: &access,
        deps: &deps,
    };
    let allowed = check.call(&step.model).await;
    let identity = identity_of(token.as_ref())?;
    Ok(json!({
        "allowed": allowed,
        "error": Value::Null,
        "warnings": Vec::<String>::new(),
        "effects": Vec::<String>::new(),
        "token": identity,
    }))
}

fn step_result(outcome: Result<(), AgentModelGrantsError>, effects: Vec<String>) -> Value {
    match outcome {
        Ok(()) => json!({
            "allowed": true,
            "error": Value::Null,
            "warnings": Vec::<String>::new(),
            "effects": effects,
            "token": Value::Null,
        }),
        Err(error) => json!({
            "allowed": false,
            "error": error_value(&error),
            "warnings": Vec::<String>::new(),
            "effects": effects,
            "token": Value::Null,
        }),
    }
}

fn token_of(step: &Step) -> Result<Option<UserAPIKeyAuth>, Box<HelperError>> {
    let Some(fields) = step.token.clone() else {
        return Ok(None);
    };
    let token = UserAPIKeyAuth::construct(AuthInput::Dict(fields)).or_value_error()?;
    Ok(Some(token))
}

// the token survives only as the identity both sides could observe through the metadata adapter
fn identity_of(token: Option<&UserAPIKeyAuth>) -> Result<Value, Box<HelperError>> {
    let Some(token) = token else {
        return Ok(Value::Null);
    };
    let serialized = serde_json::to_value(token)?;
    let team_id = serialized.get("team_id").cloned().unwrap_or(Value::Null);
    let user_id = serialized.get("user_id").cloned().unwrap_or(Value::Null);
    Ok(json!({"team_id": team_id, "user_id": user_id}))
}

fn error_value(error: &AgentModelGrantsError) -> Value {
    match error {
        AgentModelGrantsError::Denied(denial) => {
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
        AgentModelGrantsError::RecurseDepth { model } => json!({
            "class": "Exception",
            "status": Value::Null,
            "type": Value::Null,
            "param": Value::Null,
            "code": Value::Null,
            "message": format!("Unable to parse model, max fallback depth exceeded - received model: {model}"),
        }),
        lookups @ (AgentModelGrantsError::Unavailable
        | AgentModelGrantsError::Ceiling(_)
        | AgentModelGrantsError::Caller(_)
        | AgentModelGrantsError::Entity(_)) => json!({
            "class": "LookupError",
            "status": Value::Null,
            "type": Value::Null,
            "param": Value::Null,
            "code": Value::Null,
            "message": lookups.to_string(),
        }),
    }
}
