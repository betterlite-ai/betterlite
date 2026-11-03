mod tests;

use betterlite_auth::agent_policy::{
    AgentIdentityBinding, AgentKillSwitchAuth, AgentKillSwitchConfig, AgentResponse,
    ManagedAgentContext,
};
use betterlite_auth::budget_snapshots::{
    self, KeyBudgetSnapshot, OrgBudgetSnapshot, TeamBudgetSnapshot, UserBudgetSnapshot,
};
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
            Function::KillAuth => {
                serde_json::from_value::<AgentKillSwitchAuth>(input.value.clone())
                    .and_then(serde_json::to_value)
                    .map_err(CarriedPolicyRecordsError::from)
            }
            Function::KillConfig => {
                serde_json::from_value::<AgentKillSwitchConfig>(input.value.clone())
                    .and_then(serde_json::to_value)
                    .map_err(CarriedPolicyRecordsError::from)
            }
            Function::AgentResponse => serde_json::from_value::<AgentResponse>(input.value.clone())
                .and_then(serde_json::to_value)
                .map_err(CarriedPolicyRecordsError::from),
            Function::IdentityBinding => {
                serde_json::from_value::<AgentIdentityBinding>(input.value.clone())
                    .and_then(serde_json::to_value)
                    .map_err(CarriedPolicyRecordsError::from)
            }
            Function::ManagedContext => {
                serde_json::from_value::<ManagedAgentContext>(input.value.clone())
                    .and_then(serde_json::to_value)
                    .map_err(CarriedPolicyRecordsError::from)
            }
            Function::KeySnapshot => snapshot::<KeyBudgetSnapshot>(&input.value),
            Function::TeamSnapshot => snapshot::<TeamBudgetSnapshot>(&input.value),
            Function::UserSnapshot => snapshot::<UserBudgetSnapshot>(&input.value),
            Function::OrgSnapshot => snapshot::<OrgBudgetSnapshot>(&input.value),
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
