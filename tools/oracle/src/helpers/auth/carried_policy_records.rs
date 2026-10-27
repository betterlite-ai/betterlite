use betterlite_auth::{agent_policy, budget_snapshots};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

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
            Function::KillConfig => model::<agent_policy::AgentKillSwitchConfig>(&input.value),
            Function::AgentResponse => model::<agent_policy::AgentResponse>(&input.value),
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
                message: error,
            })
        })
    }
}

fn model<T>(value: &Value) -> Result<Value, String>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let model: T = serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    serde_json::to_value(model).map_err(|error| error.to_string())
}

fn kill_auth(value: &Value) -> Result<Value, String> {
    let auth: agent_policy::AgentKillSwitchAuth =
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    let mut output = serde_json::to_value(&auth).map_err(|error| error.to_string())?;
    let object = output
        .as_object_mut()
        .ok_or_else(|| String::from("kill-switch auth did not serialize as an object"))?;
    let field = match value.get("type").and_then(Value::as_str) {
        Some("bearer") => "token",
        Some("api_key") => "api_key",
        Some("basic") => "password",
        _ => return Err(String::from("kill-switch auth type is missing")),
    };
    object.insert(
        field.to_owned(),
        Value::String(auth.secret_value().to_owned()),
    );
    Ok(output)
}

fn snapshot<T>(value: &Value) -> Result<Value, String>
where
    T: budget_snapshots::BudgetSnapshot,
{
    let metadata: BTreeMap<String, Box<serde_json::value::RawValue>> =
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    let Some(snapshot) = T::from_metadata(&metadata) else {
        return Ok(serde_json::json!({"snapshot": null}));
    };
    let entries = snapshot
        .metadata_entries()
        .map_err(|error| error.to_string())?;
    serde_json::to_value(serde_json::json!({"snapshot": entries}))
        .map_err(|error| error.to_string())
}
