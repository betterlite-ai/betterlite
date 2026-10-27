use betterlite_auth::{agent_policy, budget_snapshots};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
enum CarriedPolicyRecordsError {
    #[error("failed to decode or encode carried policy record")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Snapshot(#[from] budget_snapshots::BudgetSnapshotError),
    #[error("kill-switch auth did not serialize as an object")]
    AuthNotObject,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    KillAuth,
    KillConfig,
    AgentResponse,
    IdentityBinding,
    ManagedContext,
    KeySnapshot,
    TeamSnapshot,
    UserSnapshot,
    OrgSnapshot,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    value: Value,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.function {
            Function::KillAuth => kill_auth(&input.value),
            Function::KillConfig => kill_config(&input.value),
            Function::AgentResponse => agent_response(&input.value),
            Function::IdentityBinding => model::<agent_policy::AgentIdentityBinding>(&input.value),
            Function::ManagedContext => model::<agent_policy::ManagedAgentContext>(&input.value),
            Function::KeySnapshot => snapshot::<budget_snapshots::KeyBudgetSnapshot>(&input.value),
            Function::TeamSnapshot => {
                snapshot::<budget_snapshots::TeamBudgetSnapshot>(&input.value)
            }
            Function::UserSnapshot => {
                snapshot::<budget_snapshots::UserBudgetSnapshot>(&input.value)
            }
            Function::OrgSnapshot => snapshot::<budget_snapshots::OrgBudgetSnapshot>(&input.value),
        }
        .map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "ValidationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn model<T>(value: &Value) -> Result<Value, CarriedPolicyRecordsError>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let model: T = serde_json::from_value(value.clone())?;
    Ok(serde_json::to_value(model)?)
}

fn kill_auth(value: &Value) -> Result<Value, CarriedPolicyRecordsError> {
    let auth: agent_policy::AgentKillSwitchAuth = serde_json::from_value(value.clone())?;
    Ok(serde_json::to_value(auth.exposed())?)
}

fn kill_config(value: &Value) -> Result<Value, CarriedPolicyRecordsError> {
    let config: agent_policy::AgentKillSwitchConfig = serde_json::from_value(value.clone())?;
    exposed_config(&config)
}

fn exposed_config(
    config: &agent_policy::AgentKillSwitchConfig,
) -> Result<Value, CarriedPolicyRecordsError> {
    let mut output = serde_json::to_value(config)?;
    let Some(auth) = config.auth.as_ref() else {
        return Ok(output);
    };
    let object = output
        .as_object_mut()
        .ok_or(CarriedPolicyRecordsError::AuthNotObject)?;
    object.insert(String::from("auth"), serde_json::to_value(auth.exposed())?);
    Ok(output)
}

fn agent_response(value: &Value) -> Result<Value, CarriedPolicyRecordsError> {
    let response: agent_policy::AgentResponse = serde_json::from_value(value.clone())?;
    let mut output = serde_json::to_value(response)?;
    if let Some(config) = output.get_mut("kill_switch").and_then(Value::as_object_mut) {
        let decoded: agent_policy::AgentKillSwitchConfig =
            serde_json::from_value(Value::Object(config.clone()))?;
        let exposed = exposed_config(&decoded)?;
        let exposed_object = exposed
            .as_object()
            .ok_or(CarriedPolicyRecordsError::AuthNotObject)?;
        config.clone_from(exposed_object);
    }
    Ok(output)
}

fn snapshot<T>(value: &Value) -> Result<Value, CarriedPolicyRecordsError>
where
    T: budget_snapshots::BudgetSnapshot,
{
    let metadata: BTreeMap<String, Box<serde_json::value::RawValue>> =
        serde_json::from_value(value.clone())?;
    let Some(snapshot) = T::from_metadata(&metadata) else {
        return Ok(serde_json::json!({"snapshot": null}));
    };
    let entries = snapshot.metadata_entries()?;
    Ok(serde_json::json!({"snapshot": entries}))
}
