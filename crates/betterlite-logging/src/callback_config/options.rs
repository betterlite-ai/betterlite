use std::str::FromStr as _;

use rust_decimal::Decimal;
use serde::Deserialize as _;
use serde::de::IntoDeserializer as _;
use serde::de::value::Error as ValueError;

use super::error::ValidationError;
use super::family::{SAMPLING_RATE_VARS, SPAN_SCOPE_VAR};
use super::{CallbackVars, FeatureFlags};
use crate::logging_privacy::callbacks::{SpanScope, validate_environment};

const NEWRELIC_REGIONS: [&str; 2] = ["eu", "us"];

pub fn callback_config_error(
    callback_name: Option<&str>,
    callback_vars: &CallbackVars,
    flags: FeatureFlags,
) -> Result<(), ValidationError> {
    if callback_vars.is_empty() {
        return Ok(());
    }
    arize_sampling_rate_error(callback_name, callback_vars)?;
    langfuse_environment_error(callback_vars)?;
    langfuse_span_scope_error(callback_name, callback_vars)?;
    if callback_name != Some("newrelic") {
        return Ok(());
    }
    newrelic_config_error(callback_vars, flags)
}

pub fn langfuse_environment_error(callback_vars: &CallbackVars) -> Result<(), ValidationError> {
    let Some(value) = callback_vars.get("langfuse_environment") else {
        return Ok(());
    };
    validate_environment(value).map_err(|source| ValidationError::LangfuseEnvironment {
        value: value.clone(),
        source,
    })
}

pub fn langfuse_span_scope_error(
    callback_name: Option<&str>,
    callback_vars: &CallbackVars,
) -> Result<(), ValidationError> {
    match (callback_vars.get(SPAN_SCOPE_VAR), callback_name) {
        (None, _) => Ok(()),
        (Some(_), name) if name != Some("langfuse_otel") => {
            Err(ValidationError::SpanScopeCallback {
                callback_name: name.map(str::to_owned),
            })
        }
        (Some(value), _) => SpanScope::deserialize(value.as_str().into_deserializer())
            .map(drop)
            .map_err(|_ignored: ValueError| ValidationError::SpanScopeValue {
                value: value.clone(),
            }),
    }
}

// An exponent below Decimal's 28-digit scale underflows to zero, or to the smallest negative value.
fn parse_sampling_rate(input: &str) -> Result<Decimal, rust_decimal::Error> {
    let text = input.trim();
    let Some((mantissa, exponent)) = text.split_once(['e', 'E']) else {
        return Decimal::from_str(text);
    };
    let parsed = Decimal::from_scientific(text);
    let tiny_exponent = exponent.strip_prefix('-').is_some_and(|digits| {
        !digits.is_empty() && digits.chars().all(|digit| digit.is_ascii_digit())
    });
    if parsed.is_ok() || !tiny_exponent || Decimal::from_str(mantissa).is_err() {
        return parsed;
    }
    if mantissa.starts_with('-') && mantissa.contains(|digit| matches!(digit, '1'..='9')) {
        return Ok(Decimal::new(-1, 28));
    }
    Ok(Decimal::ZERO)
}

fn sampling_rate_in_bounds(var: &str, value: &str) -> Result<(), ValidationError> {
    let rate =
        parse_sampling_rate(value).map_err(|source| ValidationError::SamplingRateUnparsable {
            var: var.to_owned(),
            value: value.to_owned(),
            source,
        })?;
    if (Decimal::ZERO..=Decimal::ONE).contains(&rate) {
        return Ok(());
    }
    Err(ValidationError::SamplingRateOutOfRange {
        var: var.to_owned(),
        value: value.to_owned(),
        rate,
    })
}

pub fn arize_sampling_rate_error(
    callback_name: Option<&str>,
    callback_vars: &CallbackVars,
) -> Result<(), ValidationError> {
    let set_rates = SAMPLING_RATE_VARS.into_iter().filter_map(|var| {
        callback_vars
            .get(var)
            .filter(|value| !value.is_empty() && *value != "None")
            .map(|value| (var, value))
    });
    for (var, value) in set_rates {
        if callback_name != Some("arize") {
            return Err(ValidationError::SamplingRateCallback {
                var: var.to_owned(),
                callback_name: callback_name.map(str::to_owned),
            });
        }
        sampling_rate_in_bounds(var, value)?;
    }
    Ok(())
}

pub fn newrelic_config_error(
    callback_vars: &CallbackVars,
    flags: FeatureFlags,
) -> Result<(), ValidationError> {
    if !callback_vars.keys().any(|key| key.starts_with("newrelic_")) {
        return Ok(());
    }
    if !flags.otel_v2_enabled {
        return Err(ValidationError::NewRelicRequiresOtelV2);
    }
    let region = callback_vars.get("newrelic_region");
    if let Some(given) = region
        && !NEWRELIC_REGIONS.contains(&given.to_lowercase().as_str())
    {
        return Err(ValidationError::NewRelicRegion {
            region: given.clone(),
        });
    }
    // A JSON null reaches this layer as the text "None", so it counts as absent like "".
    let api_key = callback_vars.get("newrelic_api_key");
    if region.is_some() && api_key.is_none_or(|key| key.is_empty() || key == "None") {
        return Err(ValidationError::NewRelicRegionNeedsKey);
    }
    Ok(())
}
