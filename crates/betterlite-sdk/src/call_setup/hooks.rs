use std::sync::Arc;

use async_trait::async_trait;
use betterlite_core::routes::CallType;
use betterlite_error::error::ErrorClass;
use betterlite_logging::callback_logger::CallbackLogger;
use betterlite_models::decisions::Json;
use serde::Deserialize as _;
use serde::de::value::{Error as ValueError, StrDeserializer};
use thiserror::Error;

use super::correlation::CallOutcome;
use crate::call_setup::RequestMap;

#[derive(Debug, Error)]
pub enum HookError {
    #[error("the {hook} deployment hook failed: {message}")]
    Failed { hook: String, message: String },
}

// Some marks a guardrail, whose error the success hook re-raises; an unnamed guardrail still counts
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guardrail {
    pub name: Option<String>,
    pub default_on: bool,
}

#[async_trait]
pub trait DeploymentHook: CallbackLogger + Send + Sync {
    fn guardrail(&self) -> Option<Guardrail> {
        None
    }

    async fn pre_call_deployment_hook(
        &self,
        _request: &mut RequestMap,
        _call_type: Option<CallType>,
    ) -> Result<Option<RequestMap>, HookError> {
        Ok(None)
    }

    async fn success_deployment_hook(
        &self,
        _request_data: &RequestMap,
        _response: &Json,
        _call_type: Option<CallType>,
    ) -> Result<Option<Json>, HookError> {
        Ok(None)
    }

    async fn failure_deployment_hook(
        &self,
        _request_data: &RequestMap,
        _error: &ErrorClass,
        _call_type: Option<CallType>,
        _fallback_depth: Option<i64>,
    ) -> Result<(), HookError> {
        Ok(())
    }
}

// An unknown call type reaches LiteLLM's hooks as None, not as an error
#[must_use]
pub fn hook_call_type(call_type: &str) -> Option<CallType> {
    let parsed: Result<CallType, ValueError> =
        CallType::deserialize(StrDeserializer::<ValueError>::new(call_type));
    parsed.ok()
}

pub async fn pre_call_deployment_hook(
    hooks: &[Arc<dyn DeploymentHook>],
    request: &RequestMap,
    call_type: &str,
) -> Result<RequestMap, HookError> {
    let typed = hook_call_type(call_type);
    let mut modified = request.clone();
    for hook in hooks {
        if let Some(replacement) = hook.pre_call_deployment_hook(&mut modified, typed).await? {
            modified = replacement;
        }
    }
    Ok(modified)
}

pub async fn success_deployment_hook(
    hooks: &[Arc<dyn DeploymentHook>],
    request_data: &RequestMap,
    response: &Json,
    call_type: &str,
) -> Result<Json, HookError> {
    let mut outcome = response.clone();
    for hook in hooks {
        let typed = hook_call_type(call_type);
        let hooked = hook
            .success_deployment_hook(request_data, &outcome, typed)
            .await;
        outcome = settle_success(hook.as_ref(), hooked, outcome)?;
    }
    Ok(outcome)
}

// A guardrail's error stops the call, while a broken logger only loses its own hook
fn settle_success(
    hook: &dyn DeploymentHook,
    hooked: Result<Option<Json>, HookError>,
    current: Json,
) -> Result<Json, HookError> {
    match hooked {
        Ok(Some(replacement)) => Ok(replacement),
        Ok(None) => Ok(current),
        Err(error) if hook.guardrail().is_some() => Err(error),
        Err(error) => {
            tracing::error!(hook = hook.callback_name(), %error, "the success deployment hook failed");
            Ok(current)
        }
    }
}

pub async fn failure_deployment_hook(
    hooks: &[Arc<dyn DeploymentHook>],
    request_data: &RequestMap,
    error: &ErrorClass,
    call_type: &str,
) {
    let typed = hook_call_type(call_type);
    let depth = request_data.get("fallback_depth").and_then(fallback_depth);
    let safe = without_attempted_targets(request_data);
    for hook in hooks {
        let failed = hook
            .failure_deployment_hook(&safe, error, typed, depth)
            .await;
        if let Err(hook_error) = failed {
            tracing::debug!(hook = hook.callback_name(), %hook_error, "the failure deployment hook failed");
        }
    }
}

pub async fn run_success_hook_on_stream(
    hooks: &[Arc<dyn DeploymentHook>],
    outcome: &mut CallOutcome,
    request_data: &RequestMap,
    call_type: &str,
) -> Result<(), HookError> {
    let CallOutcome::ChatStream(stream) = outcome else {
        return Ok(());
    };
    if !stream.converted_from_mock || hook_call_type(call_type).is_none() {
        return Ok(());
    }
    let hooked =
        success_deployment_hook(hooks, request_data, &stream.model_response, call_type).await?;
    stream.model_response = hooked;
    Ok(())
}

// The router shares this list across every fallback hop, so a callback must never reach it
fn without_attempted_targets(request_data: &RequestMap) -> RequestMap {
    request_data
        .iter()
        .filter(|(key, _)| key.as_str() != "attempted_targets")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn fallback_depth(value: &Json) -> Option<i64> {
    match value {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}
