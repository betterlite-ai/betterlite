use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::tls::Verification;
use betterlite_guardrails::ibm_detector_client::config::IbmDetectorConfig;
use betterlite_guardrails::ibm_detector_client::message::{
    detector_server_message, orchestrator_message,
};
use betterlite_guardrails::ibm_detector_client::threshold::filter_detections;
use betterlite_guardrails::ibm_detector_client::wire::Detection;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use crate::case::UpstreamAnswer;
use crate::helpers::{HelperError, HelperResult};

mod detect;
mod request;

const GUARDRAIL_NAME: &str = "ibm-detector";

pub(crate) struct Op;

type Settings = BTreeMap<String, GuardrailValue>;
type Environment = BTreeMap<String, String>;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Configure {
        settings: Settings,
        #[serde(default)]
        environment: Environment,
    },
    Detect {
        settings: Settings,
        #[serde(default)]
        environment: Environment,
        contents: Vec<String>,
        answer: Option<UpstreamAnswer>,
    },
    Filter {
        settings: Settings,
        #[serde(default)]
        environment: Environment,
        detections: Vec<Detection>,
    },
    MessageDetectorServer {
        settings: Settings,
        #[serde(default)]
        environment: Environment,
        detections: Vec<Vec<Detection>>,
    },
    MessageOrchestrator {
        settings: Settings,
        #[serde(default)]
        environment: Environment,
        detections: Vec<Detection>,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Configure {
                settings,
                environment,
            } => {
                let config = configure(settings, environment)?;
                Ok(config_value(&config))
            }
            Input::Detect {
                settings,
                environment,
                contents,
                answer,
            } => detect::detect(settings, environment, contents, answer.as_ref()),
            Input::Filter {
                settings,
                environment,
                detections,
            } => {
                let config = configure(settings, environment)?;
                serde_json::to_value(filter_detections(&config, detections))
                    .map_err(|error| failure("SerializationError", error.to_string()))
            }
            Input::MessageDetectorServer {
                settings,
                environment,
                detections,
            } => {
                let config = configure(settings, environment)?;
                Ok(json!(detector_server_message(&config, detections)))
            }
            Input::MessageOrchestrator {
                settings,
                environment,
                detections,
            } => {
                let config = configure(settings, environment)?;
                Ok(json!(orchestrator_message(&config, detections)))
            }
        }
    }
}

fn config_value(config: &IbmDetectorConfig) -> Value {
    json!({
        "guardrail_name": config.guardrail_name.clone(),
        "auth_token_present": !config.auth_token.expose_secret().is_empty(),
        "base_url": config.base_url.clone(),
        "api_url": config.api_url.clone(),
        "detector_id": config.detector_id.clone(),
        "is_detector_server": config.is_detector_server,
        "detector_params": config.detector_params.clone(),
        "extra_headers": config.extra_headers.clone(),
        "score_threshold": config.score_threshold,
        "block_on_detection": config.block_on_detection,
        "verify_ssl": config.ssl_verification == Verification::Enabled,
    })
}

fn configure(
    settings: &Settings,
    environment: &Environment,
) -> Result<IbmDetectorConfig, Box<HelperError>> {
    let mut parameters = serde_json::to_value(settings)
        .map_err(|error| failure("ConfigError", error.to_string()))?;
    if let Value::Object(map) = &mut parameters {
        map.insert(
            "guardrail".to_owned(),
            Value::String("ibm_guardrails".to_owned()),
        );
    }
    let entry = json!({"guardrail_name": GUARDRAIL_NAME, "litellm_params": parameters});
    let config = json!({"guardrails": [entry]});
    let document = serde_json::to_string(&config)
        .map_err(|error| failure("ConfigError", error.to_string()))?;
    let loaded = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&document)
    .map_err(|error| failure("ConfigError", error.to_string()))?;
    let configured = loaded
        .sections
        .ibm_guardrails
        .guardrails
        .into_iter()
        .next()
        .ok_or_else(|| failure("ConfigError", "no IBM guardrail".to_owned()))?;
    IbmDetectorConfig::try_from(configured)
        .map_err(|error| failure("ValueError", error.to_string()))
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
