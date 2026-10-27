use betterlite_auth::{agent_policy, budget_snapshots};
use serde::Deserialize;
use serde_json::{Value, value::RawValue};
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: String,
    value: Value,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input.function.as_str() {
            "kill_auth" => {
                serde_json::from_value::<agent_policy::AgentKillSwitchAuth>(input.value.clone())
                    .and_then(serde_json::to_value)
            }
            "kill_config" => {
                serde_json::from_value::<agent_policy::AgentKillSwitchConfig>(input.value.clone())
                    .and_then(serde_json::to_value)
            }
            "agent_response" => {
                serde_json::from_value::<agent_policy::AgentResponse>(input.value.clone())
                    .and_then(serde_json::to_value)
            }
            "identity_binding" => {
                serde_json::from_value::<agent_policy::AgentIdentityBinding>(input.value.clone())
                    .and_then(serde_json::to_value)
            }
            "managed_context" => {
                serde_json::from_value::<agent_policy::ManagedAgentContext>(input.value.clone())
                    .and_then(serde_json::to_value)
            }
            "key_snapshot" => snapshot::<budget_snapshots::KeyBudgetSnapshot>(&input.value),
            "team_snapshot" => snapshot::<budget_snapshots::TeamBudgetSnapshot>(&input.value),
            "user_snapshot" => snapshot::<budget_snapshots::UserBudgetSnapshot>(&input.value),
            "org_snapshot" => snapshot::<budget_snapshots::OrgBudgetSnapshot>(&input.value),
            _ => Err(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "unknown carried policy function",
            ))),
        };
        output.map_err(|error| {
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

fn snapshot<T>(value: &Value) -> Result<Value, serde_json::Error>
where
    T: budget_snapshots::BudgetSnapshot,
{
    let metadata: BTreeMap<String, Box<RawValue>> = serde_json::from_value(value.clone())?;
    let snapshot = T::from_metadata(&metadata).ok_or_else(|| {
        serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid snapshot",
        ))
    })?;
    serde_json::to_value(snapshot.metadata_entries())
}
