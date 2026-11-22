mod outcome;
mod recorder;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::health_probe_modes::response::health_check_response;
use outcome::{mode_step, tracking_step, wildcard_step};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::runtime::Builder;
const TRACKED: [&str; 7] = [
    "tags",
    "user_api_key",
    "user_api_key_hash",
    "user_api_key_alias",
    "user_api_key_team_id",
    "user_api_key_team_alias",
    "user_api_key_spend",
];
#[derive(Deserialize)]
pub(crate) struct Input {
    mode: Vec<ModeStep>,
    wildcard: Vec<WildcardStep>,
    tracking: Vec<TrackingStep>,
    response: Vec<ResponseStep>,
}
#[derive(Clone, Deserialize)]
struct Failure {
    class_name: String,
    message: String,
}
#[derive(Deserialize)]
struct ModeStep {
    mode: String,
    model: String,
    custom_llm_provider: String,
    params: serde_json::Map<String, Value>,
    prompt: Option<String>,
    inputs: Option<Vec<String>>,
    logging: bool,
    failure: Option<Failure>,
}
#[derive(Deserialize)]
struct WildcardStep {
    model: String,
    custom_llm_provider: String,
    params: serde_json::Map<String, Value>,
    cheapest: Vec<String>,
}
#[derive(Deserialize)]
struct TrackingStep {
    params: serde_json::Map<String, Value>,
}
#[derive(Deserialize)]
struct ResponseStep {
    headers: BTreeMap<String, String>,
}
async fn run_steps(input: &Input) -> Vec<Value> {
    let mut results = Vec::new();
    for step in &input.mode {
        results.push(mode_step(step).await);
    }
    for step in &input.wildcard {
        results.push(wildcard_step(step).await);
    }
    for step in &input.tracking {
        results.push(tracking_step(step));
    }
    for step in &input.response {
        let response =
            serde_json::to_value(health_check_response(&step.headers)).unwrap_or(Value::Null);
        results.push(response);
    }
    results
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().enable_all().build()?;
        let results = runtime.block_on(run_steps(input));
        Ok(Value::Array(results))
    }
}
