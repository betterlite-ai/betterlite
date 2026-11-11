use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;
use serde::Deserialize;

use crate::batch::logger::BatchSettings;
use crate::datadog::tagging::Environment;
use crate::exporter_options::options::CustomLoggerOptions;

use super::error::DatadogLlmObsError;

mod batch;

use batch::batch;

pub const DEFAULT_AGENT_PORT: &str = "8126";
pub const INTAKE_PATH: &str = "/api/intake/llm-obs/v1/trace/spans";

const AGENT_HOST_VARIABLE: &str = "LITELLM_DD_AGENT_HOST";
const AGENT_PORT_VARIABLE: &str = "LITELLM_DD_LLM_OBS_PORT";
const API_KEY_VARIABLE: &str = "DD_API_KEY";
const SITE_VARIABLE: &str = "DD_SITE";

// litellm.datadog_llm_observability_params holds DatadogLLMObsInitParams, whose only field is this flag
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DatadogLlmObsInitParams {
    pub turn_off_message_logging: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct DatadogLlmObsInit {
    pub dd_api_key: Option<SecretString>,
    pub dd_site: Option<String>,
    pub dd_agent_host: Option<String>,
    pub dd_agent_port: Option<String>,
    // the deprecated constructor kwarg, which litellm.datadog_llm_observability_params may override
    pub turn_off_message_logging: Option<bool>,
    pub message_logging: Option<bool>,
    // CustomBatchLogger's constructor forwards both; the proxy builds the logger with neither
    pub batch_size: Option<usize>,
    pub flush_interval_seconds: Option<f64>,
    pub params: Option<DatadogLlmObsInitParams>,
    // litellm_settings.turn_off_message_logging, the global flag the request check falls back to
    pub global_turn_off: bool,
}

#[derive(Clone, Debug)]
pub struct DatadogLlmObsSettings {
    pub intake_url: String,
    pub api_key: Option<SecretString>,
    pub batch: BatchSettings,
    pub options: CustomLoggerOptions,
    pub environment: Environment,
    pub global_turn_off: bool,
}

impl DatadogLlmObsSettings {
    pub fn resolve(
        init: &DatadogLlmObsInit,
        environment: &dyn EnvironmentLookup,
    ) -> Result<Self, DatadogLlmObsError> {
        let tags = Environment::from_lookup(environment)?;
        let (endpoint_url, api_key) = endpoint(init, environment)?;
        Ok(Self {
            intake_url: base_url(&tags, endpoint_url),
            api_key,
            batch: batch(init, environment)?,
            options: options(init),
            environment: tags,
            global_turn_off: init.global_turn_off,
        })
    }
}

fn base_url(tags: &Environment, endpoint_url: String) -> String {
    tags.base_url
        .as_deref()
        .filter(|base| !base.is_empty())
        .map_or(endpoint_url, |base| format!("{base}{INTAKE_PATH}"))
}

fn options(init: &DatadogLlmObsInit) -> CustomLoggerOptions {
    let param_flag = init
        .params
        .as_ref()
        .and_then(|params| params.turn_off_message_logging);
    CustomLoggerOptions {
        turn_off_message_logging: param_flag
            .or(init.turn_off_message_logging)
            .unwrap_or(false),
        message_logging: init.message_logging.unwrap_or(true),
    }
}

// the agent is checked first, so agent mode needs no API key; direct API mode needs both credentials
fn endpoint(
    init: &DatadogLlmObsInit,
    environment: &dyn EnvironmentLookup,
) -> Result<(String, Option<SecretString>), DatadogLlmObsError> {
    let api_key = init
        .dd_api_key
        .clone()
        .or_else(|| credential(environment, API_KEY_VARIABLE));
    let agent_host = init.dd_agent_host.clone().or_else(|| {
        environment
            .lookup(AGENT_HOST_VARIABLE)
            .filter(|value| !value.is_empty())
    });
    let Some(host) = agent_host else {
        let key = api_key.ok_or(DatadogLlmObsError::MissingApiKey)?;
        let site = init
            .dd_site
            .clone()
            .or_else(|| {
                environment
                    .lookup(SITE_VARIABLE)
                    .filter(|value| !value.is_empty())
            })
            .ok_or(DatadogLlmObsError::MissingSite)?;
        return Ok((format!("https://api.{site}{INTAKE_PATH}"), Some(key)));
    };
    let port = init
        .dd_agent_port
        .clone()
        .or_else(|| {
            environment
                .lookup(AGENT_PORT_VARIABLE)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_AGENT_PORT.to_owned());
    Ok((format!("http://{host}:{port}{INTAKE_PATH}"), api_key))
}

fn credential(environment: &dyn EnvironmentLookup, name: &str) -> Option<SecretString> {
    environment
        .lookup(name)
        .filter(|value| !value.is_empty())
        .map(SecretString::from)
}
