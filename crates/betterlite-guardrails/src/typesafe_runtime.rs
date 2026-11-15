mod apply;
pub mod error;
mod messages;
mod records;
mod tests;

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::anthropic_history::groups::group_tool_exchanges;
use betterlite_prompts::arguments::json::Json;
use secrecy::{ExposeSecret as _, SecretString};

use crate::generic_guardrail_contracts::{GuardrailInputType, UnreachableFallback};
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::third_party_init_b::typesafe::TypeSafeArgs;
use crate::typesafe::typesafe_config::{
    MaxResultChars, MinCharsToEvaluate, RelevanceThreshold, TypeSafeConfigError, TypeSafeSettings,
};
use crate::typesafe::typesafe_protected_indices::compression_protected_indices;
use crate::typesafe::{DEFAULT_API_BASE, DEFAULT_JEV_TIMEOUT, DEFAULT_MODEL, TypeSafeGuardrail};
use error::TypeSafeRuntimeError;

// The two names LiteLLM reads through get_secret_str before it builds the class
#[derive(Debug, Clone)]
pub struct TypeSafeDefaults {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
}

#[derive(Debug)]
pub struct TypeSafeGuard {
    base: GuardrailBase,
    runtime: TypeSafeGuardrail,
    clock: fn() -> SystemTime,
}

impl TypeSafeGuard {
    pub fn new(
        args: &TypeSafeArgs,
        defaults: TypeSafeDefaults,
        base: GuardrailBase,
        client: reqwest::Client,
    ) -> Result<Self, TypeSafeRuntimeError> {
        let api_key = resolve_key(args.api_key.clone(), defaults.api_key)?;
        let settings = resolve_settings(args, defaults.api_base)?;
        let timeout = resolve_timeout(base.settings.timeout)?;
        let runtime = TypeSafeGuardrail {
            settings,
            api_key,
            client,
            timeout,
            content_to_text: messages::content_text,
            get_protected_indices: compression_protected_indices,
            group_tool_exchanges,
        };
        Ok(Self {
            base,
            runtime,
            clock: SystemTime::now,
        })
    }
}

impl GuardrailContract for TypeSafeGuard {
    async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        if input_type == GuardrailInputType::Response {
            return Ok(inputs);
        }
        let rows: Vec<Json> = inputs
            .structured_messages
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(messages::row)
            .collect();
        apply::compact(self, inputs, &rows, request_data, logging_obj).await
    }
}

fn resolve_key(
    argument: Option<SecretString>,
    default: Option<SecretString>,
) -> Result<SecretString, TypeSafeRuntimeError> {
    argument
        .or(default)
        .filter(|key| !key.expose_secret().is_empty())
        .ok_or(TypeSafeRuntimeError::MissingApiKey)
}

fn resolve_settings(
    args: &TypeSafeArgs,
    default_base: Option<String>,
) -> Result<TypeSafeSettings, TypeSafeRuntimeError> {
    let api_base = args
        .api_base
        .clone()
        .or(default_base)
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    Ok(TypeSafeSettings {
        api_base: api_base.trim_end_matches('/').to_owned(),
        model: args
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
        unreachable_fallback: resolve_fallback(args.unreachable_fallback.as_deref()),
        relevance_threshold: args
            .relevance_threshold
            .map(RelevanceThreshold::try_from)
            .transpose()?
            .unwrap_or_default(),
        min_chars_to_evaluate: args
            .min_chars_to_evaluate
            .map(MinCharsToEvaluate::try_from)
            .transpose()?
            .unwrap_or_default(),
        max_result_chars_in_state: max_result_chars(args.max_result_chars_in_state)?,
    })
}

// LiteLLM reads every value other than the exact string "fail_closed" as fail_open
fn resolve_fallback(fallback: Option<&str>) -> UnreachableFallback {
    if fallback == Some("fail_closed") {
        return UnreachableFallback::FailClosed;
    }
    UnreachableFallback::FailOpen
}

fn resolve_timeout(timeout: Option<f64>) -> Result<Duration, TypeSafeRuntimeError> {
    timeout.map_or(Ok(DEFAULT_JEV_TIMEOUT), |seconds| {
        Duration::try_from_secs_f64(seconds)
            .map_err(|source| TypeSafeRuntimeError::Timeout { seconds, source })
    })
}

fn max_result_chars(value: Option<i64>) -> Result<MaxResultChars, TypeSafeRuntimeError> {
    let Some(limit) = value else {
        return Ok(MaxResultChars::default());
    };
    let chars = usize::try_from(limit).map_err(|_negative| TypeSafeConfigError::MaxResultChars)?;
    MaxResultChars::try_from(chars).map_err(TypeSafeRuntimeError::from)
}
