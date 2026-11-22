use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::types::Config;
use betterlite_models::decisions::Json;
use betterlite_router::router_predicates::JsonMap;
use betterlite_server::router_settings::response::{fields_response, settings_response};
use betterlite_server::router_settings::view::{current_values, settings_store};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

pub type Output = Json;

// the case names the endpoint and the router_settings document the proxy would have loaded
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Input {
    pub endpoint: Endpoint,
    pub router_settings: RouterSettings,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Endpoint {
    #[default]
    Settings,
    Fields,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RouterSettings {
    pub routing_strategy: Option<String>,
    pub routing_strategy_args: Option<JsonMap>,
    pub routing_groups: Option<Vec<JsonMap>>,
    pub num_retries: Option<i64>,
    pub timeout: Option<Number>,
    pub stream_timeout: Option<Number>,
    pub max_fallbacks: Option<i64>,
    pub fallbacks: Option<Vec<JsonMap>>,
    pub context_window_fallbacks: Option<Vec<JsonMap>>,
    pub content_policy_fallbacks: Option<Vec<JsonMap>>,
    pub allowed_fails: Option<i64>,
    pub cooldown_time: Option<Number>,
    pub retry_after: Option<Number>,
    pub retry_policy: Option<BTreeMap<String, i64>>,
    pub model_group_alias: Option<BTreeMap<String, String>>,
    pub enable_pre_call_checks: Option<bool>,
    pub default_litellm_params: Option<JsonMap>,
    pub set_verbose: Option<bool>,
    pub default_max_parallel_requests: Option<i64>,
    pub enable_tag_filtering: Option<bool>,
    pub tag_filtering_match_any: Option<bool>,
    pub disable_cooldowns: Option<bool>,
}

// a JSON number is finite, so exact equality over the parsed value is total
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Number(pub f64);

impl Eq for Number {}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let config = load(&input.router_settings)?;
    let serialized = match input.endpoint {
        Endpoint::Settings => {
            let response = settings_response(current_values(&config), &settings_store(&config));
            serde_json::to_string(&response).map_err(invalid)?
        }
        Endpoint::Fields => serde_json::to_string(&fields_response()).map_err(invalid)?,
    };
    let answer = serde_json::from_str(&serialized).map_err(invalid)?;
    Ok(ProbeOutput::RouterSettings(answer))
}

fn load(settings: &RouterSettings) -> Result<Config, OperationError> {
    let loader = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    };
    loader.load(&document(settings)?).map_err(invalid)
}

// the case names the settings it sets, and a setting it leaves out stays out of the document
fn document(settings: &RouterSettings) -> Result<String, OperationError> {
    let value = serde_json::to_value(settings).map_err(invalid)?;
    let document = JsonValue::Object(
        [(String::from("router_settings"), without_nulls(value))]
            .into_iter()
            .collect(),
    );
    serde_json::to_string(&document).map_err(invalid)
}

fn without_nulls(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(entries) => JsonValue::Object(
            entries
                .into_iter()
                .filter(|(_, value)| *value != JsonValue::Null)
                .map(|(key, value)| (key, without_nulls(value)))
                .collect(),
        ),
        JsonValue::Array(items) => JsonValue::Array(items.into_iter().map(without_nulls).collect()),
        other => other,
    }
}

fn invalid<E: Display>(error: E) -> OperationError {
    OperationError::InvalidRequest {
        message: error.to_string(),
    }
}
