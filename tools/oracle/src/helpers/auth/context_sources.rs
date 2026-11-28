mod budget;
mod names;
mod settings;
mod sso;

use std::collections::BTreeMap;

use betterlite_auth::auth_context_sources::sso::abbreviate_api_key;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::Builder;

pub(crate) struct Op;

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    #[default]
    Sso,
    Abbreviate,
    BudgetNames,
    Claude,
    RouterSettings,
    RouterAlias,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LookupMode {
    #[default]
    Present,
    Absent,
    Error,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    scenarios: Vec<sso::SsoScenario>,
    api_key: Option<String>,
    minimum_custom_key_length: Option<i64>,
    model: Option<budget::BudgetModel>,
    requested: Option<String>,
    global_aliases: BTreeMap<String, String>,
    key_aliases: Option<Json>,
    team_model_aliases: Option<Json>,
    hierarchical_aliases: Option<Json>,
    router_model_group_alias: Vec<String>,
    router_model_ids: Vec<String>,
    candidate_ids: BTreeMap<String, Vec<String>>,
    team_id: Option<String>,
    key_settings: Option<Json>,
    team_settings: Option<BTreeMap<String, JsonData>>,
    team_lookup: Option<LookupMode>,
    route: String,
    #[serde(flatten)]
    routing: RoutingState,
    #[serde(flatten)]
    body: BodyState,
    requested_model: Option<String>,
    settings: Option<Json>,
    client_requested_model: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RoutingState {
    router_available: bool,
    marker_set: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BodyState {
    passthrough: bool,
    raw_body_available: bool,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.function {
            Function::Sso => Ok(json!(
                input
                    .scenarios
                    .iter()
                    .map(sso::scenario_answer)
                    .collect::<Vec<Value>>()
            )),
            Function::Abbreviate => {
                let api_key = SecretString::from(input.api_key.clone().unwrap_or_default());
                Ok(json!(abbreviate_api_key(
                    &api_key,
                    input.minimum_custom_key_length.unwrap_or_default()
                )))
            }
            Function::BudgetNames => Ok(budget::budget_answer(input)),
            Function::Claude => Ok(names::claude_answer(input)),
            Function::RouterSettings => {
                let runtime = Builder::new_current_thread().enable_all().build()?;
                Ok(json!(runtime.block_on(settings::resolved_settings(input))))
            }
            Function::RouterAlias => {
                let runtime = Builder::new_current_thread().enable_all().build()?;
                Ok(runtime.block_on(settings::alias_rewrite(input)))
            }
        }
    }
}
