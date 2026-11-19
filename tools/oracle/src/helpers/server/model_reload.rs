mod answers;
mod support;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use tokio::runtime::Builder;

use self::answers::answer;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    environment: BTreeMap<String, String>,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    ModelInfoAsMapping(MappingCall),
    ReloadServingVerdict(VerdictCall),
    RaiseIfReloadDegradedServing(RaiseCall),
    ClearCache(ClearCacheCall),
}

#[derive(Debug, Deserialize)]
struct MappingCall {
    model_info: Json,
}

#[derive(Debug, Deserialize)]
struct WrittenInput {
    id: String,
    model_info: Json,
}

#[derive(Debug, Deserialize)]
struct LiveInput {
    captured: Option<Vec<String>>,
    router_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct VerdictCall {
    obligation: ObligationInput,
    before: Vec<String>,
    written: Vec<WrittenInput>,
    still_desired: Option<Vec<String>>,
    live: LiveInput,
}

#[derive(Debug, Deserialize)]
struct RaiseCall {
    action: String,
    before: Vec<String>,
    written: Vec<WrittenInput>,
    still_desired: Option<Vec<String>>,
    live: LiveInput,
}

#[derive(Debug, Deserialize)]
struct ClearCacheCall {
    router_present: bool,
    database_present: bool,
    models: Vec<ModelEntry>,
    strategy_names: Vec<String>,
    outcome: OutcomeInput,
    reload_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    model_name: String,
    litellm_params: JsonObject,
    model_info: JsonObject,
}

#[derive(Debug, Deserialize)]
struct OutcomeInput {
    still_desired: Option<Vec<String>>,
    live_after: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ObligationInput {
    MustServe,
    MustNotDegrade,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error.to_string()))?;
        runtime.block_on(answer(input))
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
