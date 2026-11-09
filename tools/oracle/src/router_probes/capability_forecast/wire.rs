use betterlite_router::capability_forecast::config::ResponseFormat;
use betterlite_router::capability_forecast::verdict::{Boundary, Rule};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Payload {
    pub(super) function: Function,
    #[serde(default)]
    pub(super) mode: Option<Format>,
    #[serde(default)]
    pub(super) reply: Option<Reply>,
    #[serde(default)]
    pub(super) config: Option<ConfigInput>,
    #[serde(default)]
    pub(super) calibration: Option<CalibrationInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Function {
    ResponseFormat,
    SystemPrompt,
    Evaluate,
    ValidateConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Format {
    JsonSchema,
    JsonObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Reply {
    Text {
        content: String,
    },
    Padded {
        content: String,
        pad: String,
        pad_len: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfigInput {
    pub(super) efficient_tier: String,
    pub(super) capable_tier: String,
    pub(super) base_threshold: String,
    #[serde(default)]
    pub(super) threshold_step: Option<String>,
    #[serde(default)]
    pub(super) max_output_tokens: Option<u32>,
    #[serde(default)]
    pub(super) calibration: Option<CalibrationInput>,
    #[serde(default)]
    pub(super) response_format: Option<Format>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CalibrationInput {
    pub(super) version: String,
    pub(super) slope: String,
    pub(super) intercept: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Fields {
    pub(super) response_format: Option<String>,
    pub(super) system_prompt: Option<String>,
    pub(super) route: Option<RouteOutcome>,
    pub(super) config: Option<ConfigOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum RouteOutcome {
    Routed {
        tier: String,
        threshold: String,
        crux: String,
        p_solve: String,
        meets: bool,
        primary_rule: Rule,
        capability_boundary: Boundary,
        calibration_version: Option<String>,
    },
    Rejected,
    InvalidConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ConfigOutcome {
    Valid {
        efficient_tier: String,
        capable_tier: String,
        base_threshold: String,
        threshold_step: String,
        max_output_tokens: u32,
        response_format: ResponseFormat,
        calibration_version: Option<String>,
    },
    Invalid,
}
