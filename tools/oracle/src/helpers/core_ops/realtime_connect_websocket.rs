mod fixtures;

use betterlite_config::api::realtime_connect::RealtimeConnectionSettings;
use betterlite_core::api::realtime_connect::RealtimeConnectParams;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;
use tokio::runtime::Builder as RuntimeBuilder;

use fixtures::{TokenSetup, run_scenario};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = RuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let mut answers = Vec::with_capacity(input.scenarios.len());
        for scenario in &input.scenarios {
            answers.push(runtime.block_on(run_scenario(scenario))?);
        }
        Ok(json!({ "scenarios": answers }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scenario {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) params: RealtimeConnectParams,
    #[serde(default)]
    pub(crate) query_params: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub(crate) settings: RealtimeConnectionSettings,
    #[serde(default)]
    pub(crate) token: Option<TokenSetup>,
}
