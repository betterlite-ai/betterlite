use std::ops::RangeInclusive;

use betterlite_config::sections::guardrail_identity::item::GuardrailFlag;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use secrecy::SecretString;

use super::error::ThirdPartyInitBError;
use super::{InitResult, ParamsView, decode, unreachable_fallback_text};

mod tests;

const RELEVANCE_BOUNDS: RangeInclusive<f64> = 0.0..=1.0;

#[derive(Debug, Clone)]
pub struct TypeSafeArgs {
    pub name: String,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub model: Option<String>,
    pub relevance_threshold: Option<f64>,
    pub min_chars_to_evaluate: Option<i64>,
    pub max_result_chars_in_state: Option<i64>,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
    pub unreachable_fallback: Option<String>,
}

pub fn initialize_typesafe(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> InitResult<TypeSafeArgs> {
    let name = guardrail_name.ok_or(ThirdPartyInitBError::MissingTypesafeName)?;
    let extra = &params.base.extra;
    let view = ParamsView::parse(extra)?;
    let (relevance_threshold, min_chars_to_evaluate, max_result_chars_in_state) =
        optional_tuning(&view)?;
    let sent = params
        .base
        .sent_fields()
        .map_err(|source| ThirdPartyInitBError::Params { source })?;
    let unreachable_fallback = if sent.contains_key("unreachable_fallback") {
        Some(unreachable_fallback_text(params.base.fields.unreachable_fallback).to_owned())
    } else {
        None
    };
    Ok(TypeSafeArgs {
        name: name.to_owned(),
        api_base: params.base.fields.api_base.clone(),
        api_key: params.base.fields.api_key.clone(),
        model: params.base.fields.model.clone(),
        relevance_threshold,
        min_chars_to_evaluate,
        max_result_chars_in_state,
        event_hook: checked_mode(&params.mode)?,
        default_on: params.base.fields.default_on.unwrap_or_default(),
        timeout: params.base.fields.timeout,
        unreachable_fallback,
    })
}

// TypeSafeGuardrailOptionalParams validates these three in _optional_params before the class is built
fn optional_tuning(view: &ParamsView<'_>) -> InitResult<(Option<f64>, Option<i64>, Option<i64>)> {
    let relevance_threshold = decode(
        "relevance_threshold",
        view.optional
            .and_then(|entries| entries.get("relevance_threshold")),
    )?;
    if relevance_threshold.is_some_and(|threshold| !RELEVANCE_BOUNDS.contains(&threshold)) {
        return Err(ThirdPartyInitBError::InvalidParam {
            name: "relevance_threshold".to_owned(),
        });
    }
    let min_chars_to_evaluate = decode(
        "min_chars_to_evaluate",
        view.optional
            .and_then(|entries| entries.get("min_chars_to_evaluate")),
    )?;
    if min_chars_to_evaluate.is_some_and(|chars| chars < 0) {
        return Err(ThirdPartyInitBError::InvalidParam {
            name: "min_chars_to_evaluate".to_owned(),
        });
    }
    let max_result_chars_in_state = decode(
        "max_result_chars_in_state",
        view.optional
            .and_then(|entries| entries.get("max_result_chars_in_state")),
    )?;
    if max_result_chars_in_state.is_some_and(|chars| chars < 1) {
        return Err(ThirdPartyInitBError::InvalidParam {
            name: "max_result_chars_in_state".to_owned(),
        });
    }
    Ok((
        relevance_threshold,
        min_chars_to_evaluate,
        max_result_chars_in_state,
    ))
}

// LiteLLM's _coerce_event_hook turns every name in the mode into a GuardrailEventHooks member
fn checked_mode(mode: &LitellmParamsMode) -> InitResult<LitellmParamsMode> {
    match mode {
        LitellmParamsMode::One(hook) => check_hook(hook)?,
        LitellmParamsMode::Many(hooks) => {
            hooks.iter().map(String::as_str).try_for_each(check_hook)?;
        }
        LitellmParamsMode::Tagged(_) => {}
    }
    Ok(mode.clone())
}

fn check_hook(hook: &str) -> InitResult<()> {
    let encoded =
        serde_json::to_vec(hook).map_err(|_not_json| ThirdPartyInitBError::InvalidEventHook {
            hook: hook.to_owned(),
        })?;
    match serde_json::from_slice::<GuardrailEventHooks>(&encoded) {
        Ok(_known) => Ok(()),
        Err(_unknown) => Err(ThirdPartyInitBError::InvalidEventHook {
            hook: hook.to_owned(),
        }),
    }
}
