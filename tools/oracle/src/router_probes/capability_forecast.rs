use betterlite_router::capability_forecast::config::{Calibration, Config, ResponseFormat};
use betterlite_router::capability_forecast::forecast::route;
use betterlite_router::capability_forecast::prompt::{response_format, system_prompt};
use betterlite_router::capability_forecast::verdict::{Boundary, Rule};
use serde::{Deserialize, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

const MAX_PADDED_BYTES: u64 = 0x0020_0000;
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 4096;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub function: Function,
    #[serde(default)]
    pub mode: Option<Format>,
    #[serde(default)]
    pub reply: Option<Reply>,
    #[serde(default)]
    pub config: Option<ConfigInput>,
    #[serde(default)]
    pub calibration: Option<CalibrationInput>,
    #[serde(default)]
    pub probabilities: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Function {
    ResponseFormat,
    SystemPrompt,
    Evaluate,
    Calibrate,
    ValidateConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    JsonSchema,
    JsonObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
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
pub struct ConfigInput {
    pub efficient_tier: String,
    pub capable_tier: String,
    pub base_threshold: String,
    #[serde(default)]
    pub threshold_step: Option<String>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub calibration: Option<CalibrationInput>,
    #[serde(default)]
    pub response_format: Option<Format>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationInput {
    pub version: String,
    pub slope: String,
    pub intercept: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub response_format: Option<String>,
    pub system_prompt: Option<String>,
    pub route: Option<RouteOutcome>,
    pub config: Option<ConfigOutcome>,
    pub calibrated: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RouteOutcome {
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
pub enum ConfigOutcome {
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

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let payload = &input.0;
    let mut fields = Fields::default();
    match payload.function {
        Function::ResponseFormat => {
            let mode = require(payload.mode.as_ref(), "mode")?;
            fields.response_format = Some(render_format(*mode)?);
        }
        Function::SystemPrompt => {
            let mode = require(payload.mode.as_ref(), "mode")?;
            fields.system_prompt = Some(render_prompt(*mode)?);
        }
        Function::Evaluate => {
            let config = require(payload.config.as_ref(), "config")?;
            let reply = require(payload.reply.as_ref(), "reply")?;
            fields.route = Some(evaluate(config, reply)?);
        }
        Function::Calibrate => {
            let calibration = require(payload.calibration.as_ref(), "calibration")?;
            let probabilities = require(payload.probabilities.as_ref(), "probabilities")?;
            fields.calibrated = calibrated(calibration, probabilities)?;
        }
        Function::ValidateConfig => {
            let config = require(payload.config.as_ref(), "config")?;
            fields.config = Some(validate_config(config)?);
        }
    }
    Ok(ProbeOutput::CapabilityForecast(Output(Box::new(fields))))
}

fn require<'value, T>(value: Option<&'value T>, name: &str) -> Result<&'value T, OperationError> {
    value.ok_or_else(|| OperationError::InvalidRequest {
        message: format!("{name} is required for this function"),
    })
}

const fn response_format_mode(mode: Format) -> ResponseFormat {
    match mode {
        Format::JsonSchema => ResponseFormat::JsonSchema,
        Format::JsonObject => ResponseFormat::JsonObject,
    }
}

fn render_format(mode: Format) -> Result<String, OperationError> {
    let body = response_format(response_format_mode(mode));
    serde_json::to_string(&body).map_err(|source| OperationError::InvalidRequest {
        message: source.to_string(),
    })
}

fn render_prompt(mode: Format) -> Result<String, OperationError> {
    system_prompt(response_format_mode(mode)).map_err(|source| OperationError::InvalidRequest {
        message: source.to_string(),
    })
}

fn reply_bytes(reply: &Reply) -> Result<Vec<u8>, OperationError> {
    match reply {
        Reply::Text { content } => Ok(content.as_bytes().to_vec()),
        Reply::Padded {
            content,
            pad,
            pad_len,
        } => {
            if pad.is_empty() || !pad.is_ascii() || *pad_len > MAX_PADDED_BYTES {
                return Err(OperationError::InvalidRequest {
                    message: "a padded reply needs an ASCII pad and at most 2097152 pad bytes"
                        .to_owned(),
                });
            }
            let count =
                usize::try_from(*pad_len).map_err(|_error| OperationError::InvalidRequest {
                    message: format!("pad_len {pad_len} does not fit this platform"),
                })?;
            let mut builder = content.clone();
            builder.push_str(&pad.repeat(count));
            Ok(builder.into_bytes())
        }
    }
}

fn number(text: &str, field: &str) -> Result<f64, OperationError> {
    text.parse::<f64>()
        .map_err(|_error| OperationError::InvalidRequest {
            message: format!("{field} is not a number: {text}"),
        })
}

fn calibration_config(input: &CalibrationInput) -> Result<Calibration, OperationError> {
    Ok(Calibration {
        version: input.version.clone(),
        slope: number(&input.slope, "calibration.slope")?,
        intercept: number(&input.intercept, "calibration.intercept")?,
    })
}

fn draft_config(input: &ConfigInput) -> Result<Config, OperationError> {
    let threshold_step = match &input.threshold_step {
        Some(text) => number(text, "threshold_step")?,
        None => 0.0_f64,
    };
    Ok(Config {
        efficient_tier: input.efficient_tier.clone(),
        capable_tier: input.capable_tier.clone(),
        base_threshold: number(&input.base_threshold, "base_threshold")?,
        threshold_step,
        max_output_tokens: input.max_output_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS),
        calibration: input
            .calibration
            .as_ref()
            .map(calibration_config)
            .transpose()?,
        response_format: input
            .response_format
            .map_or(ResponseFormat::JsonSchema, response_format_mode),
    })
}

fn evaluate(config: &ConfigInput, reply: &Reply) -> Result<RouteOutcome, OperationError> {
    let Ok(checked) = draft_config(config)?.validate() else {
        return Ok(RouteOutcome::InvalidConfig);
    };
    let Ok(routed) = route(&checked, &reply_bytes(reply)?) else {
        return Ok(RouteOutcome::Rejected);
    };
    Ok(RouteOutcome::Routed {
        tier: routed.tier,
        threshold: format!("{:.6}", routed.forecast.threshold),
        crux: routed.forecast.verdict.crux,
        p_solve: format!("{:.6}", routed.forecast.p_solve),
        meets: routed.meets_threshold,
        primary_rule: routed.forecast.verdict.primary_rule,
        capability_boundary: routed.forecast.verdict.capability_boundary,
        calibration_version: routed.forecast.calibration_version,
    })
}

fn calibrated(
    calibration: &CalibrationInput,
    probabilities: &[String],
) -> Result<Option<Vec<String>>, OperationError> {
    let draft = Config {
        efficient_tier: "SIMPLE".to_owned(),
        capable_tier: "REASONING".to_owned(),
        base_threshold: 0.5_f64,
        threshold_step: 0.0_f64,
        max_output_tokens: DEFAULT_MAX_OUTPUT_TOKENS,
        calibration: Some(calibration_config(calibration)?),
        response_format: ResponseFormat::JsonSchema,
    };
    let Ok(checked) = draft.validate() else {
        return Ok(None);
    };
    let mut values = Vec::with_capacity(probabilities.len());
    for probability in probabilities {
        let reply = format!(
            r#"{{"crux":"x","primary_rule":"SUP-1","capability_boundary":"supported","p_solve":{probability}}}"#
        );
        let Ok(routed) = route(&checked, reply.as_bytes()) else {
            return Err(OperationError::InvalidRequest {
                message: format!("probability is not a number in [0, 1]: {probability}"),
            });
        };
        values.push(format!("{:.6}", routed.forecast.p_solve));
    }
    Ok(Some(values))
}

fn validate_config(input: &ConfigInput) -> Result<ConfigOutcome, OperationError> {
    let Ok(checked) = draft_config(input)?.validate() else {
        return Ok(ConfigOutcome::Invalid);
    };
    Ok(ConfigOutcome::Valid {
        efficient_tier: checked.efficient_tier,
        capable_tier: checked.capable_tier,
        base_threshold: format!("{:.6}", checked.base_threshold),
        threshold_step: format!("{:.6}", checked.threshold_step),
        max_output_tokens: checked.max_output_tokens,
        response_format: checked.response_format,
        calibration_version: checked.calibration.map(|calibration| calibration.version),
    })
}
