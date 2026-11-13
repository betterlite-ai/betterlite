use betterlite_router::capability_forecast::config::{Calibration, Config, ResponseFormat};
use betterlite_router::capability_forecast::forecast::route;
use betterlite_router::capability_forecast::prompt::{response_format, system_prompt};
use serde::{Deserialize, Serialize};

use self::wire::{
    CalibrationInput, ConfigInput, ConfigOutcome, Fields, Format, Function, Reply, RouteOutcome,
};
use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

mod wire;

const MAX_PADDED_BYTES: u64 = 0x0020_0000;
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 4096;

// the enum in mod.rs would otherwise carry every field of this payload inline in its largest variant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Input(Box<wire::Payload>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Output(Box<Fields>);

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
    let Some(checked) = draft_config(config)?.validate().ok() else {
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

fn validate_config(input: &ConfigInput) -> Result<ConfigOutcome, OperationError> {
    draft_config(input)?
        .validate()
        .ok()
        .map_or(Ok(ConfigOutcome::Invalid), |checked| {
            Ok(valid_config(checked))
        })
}

fn valid_config(checked: Config) -> ConfigOutcome {
    ConfigOutcome::Valid {
        calibration_version: checked.calibration.map(|calibration| calibration.version),
        base_threshold: format!("{:.6}", checked.base_threshold),
        threshold_step: format!("{:.6}", checked.threshold_step),
        max_output_tokens: checked.max_output_tokens,
        response_format: checked.response_format,
        efficient_tier: checked.efficient_tier,
        capable_tier: checked.capable_tier,
    }
}
