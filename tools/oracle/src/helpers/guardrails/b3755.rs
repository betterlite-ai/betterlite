use betterlite_config::sections::guardrail_params::records::LakeraCategoryThresholds;
use betterlite_guardrails::lakera::response::LakeraModerationResponse;
use betterlite_guardrails::lakera::{
    LegacyLakeraModeration, LegacyLakeraModerationParams, ModerationCheck,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

// LiteLLM's LakeraCategoryThresholds is total=False, so a threshold the case leaves out stays unset
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Thresholds {
    prompt_injection: Option<f64>,
    jailbreak: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub(crate) enum Function {
    #[serde(rename = "lakera_moderation_threshold_check")]
    LakeraModerationThresholdCheck,
}

// a plain struct, since an internally tagged enum buffers the response and the raw JSON it echoes cannot be buffered
#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    #[serde(rename = "function")]
    _function: Function,
    category_thresholds: Option<Thresholds>,
    response: LakeraModerationResponse,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let threshold_record =
            input
                .category_thresholds
                .map(|thresholds: Thresholds| LakeraCategoryThresholds {
                    prompt_injection: thresholds.prompt_injection,
                    jailbreak: thresholds.jailbreak,
                });
        let lakera = LegacyLakeraModeration::new(
            LegacyLakeraModerationParams {
                moderation_check: Some(ModerationCheck::PreCall),
                category_thresholds: threshold_record,
                api_base: Some("http://lakera.test".to_owned()),
                api_key: Some(SecretString::from("test")),
            },
            |_key| String::new(),
            reqwest::Client::default,
        );
        let rejected = lakera.check_response_flagged(&input.response).map_or(
            Value::Null,
            |rejection| json!({"status": rejection.status.as_u16(), "detail": rejection.detail}),
        );
        Ok(json!({ "rejected": rejected }))
    }
}
