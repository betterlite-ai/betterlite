use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize as _;
use serde::de::IntoDeserializer as _;
use serde::de::value::{Error as ValueError, StrDeserializer};

use crate::guardrail_initializers::extras;

use super::error::RepelloLifecycleError;
use super::{
    API_BASE_ENV, API_KEY_ENV, DEFAULT_API_BASE, DEFAULT_REQUEST_TIMEOUT, FALLBACK_API_KEY_ENV,
};

const ASSET_ID_FIELD: &str = "asset_id";

#[derive(Debug, Clone)]
pub struct RepelloAIParams {
    pub guardrail_name: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub asset_id: Option<String>,
    pub unreachable_fallback: UnreachableFallback,
    pub timeout: Option<f64>,
    pub mode: LitellmParamsMode,
    pub default_on: bool,
    pub strict_modes: bool,
}

impl RepelloAIParams {
    pub fn from_litellm(
        params: &LitellmParams,
        guardrail_name: &str,
        strict_modes: bool,
    ) -> Result<Self, RepelloLifecycleError> {
        let asset_id: Option<String> = extras::read(&params.base.extra, ASSET_ID_FIELD, None)
            .map_err(|_wrong| RepelloLifecycleError::InvalidParam {
                field: ASSET_ID_FIELD,
            })?;
        Ok(Self {
            guardrail_name: Some(guardrail_name.to_owned()),
            api_key: params.base.fields.api_key.clone(),
            api_base: params.base.fields.api_base.clone(),
            asset_id,
            unreachable_fallback: params.base.fields.unreachable_fallback,
            timeout: params.base.fields.timeout,
            mode: params.mode.clone(),
            default_on: params.base.fields.default_on.unwrap_or_default(),
            strict_modes,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RepelloAISettings {
    pub api_key: SecretString,
    pub api_base: String,
    pub asset_id: String,
    pub unreachable_fallback: UnreachableFallback,
    pub timeout: Option<Duration>,
    pub request_timeout: Duration,
}

impl RepelloAISettings {
    pub fn resolve<E>(params: &RepelloAIParams, env: &E) -> Result<Self, RepelloLifecycleError>
    where
        E: EnvironmentLookup,
    {
        let api_key = resolved_api_key(params, env).ok_or(RepelloLifecycleError::MissingApiKey)?;
        let asset_id = params
            .asset_id
            .clone()
            .filter(|asset| !asset.is_empty())
            .ok_or(RepelloLifecycleError::MissingAssetId)?;
        let timeout = params
            .timeout
            .map(|seconds| {
                Duration::try_from_secs_f64(seconds)
                    .map_err(|source| RepelloLifecycleError::Timeout { seconds, source })
            })
            .transpose()?;
        let api_base = params
            .api_base
            .as_deref()
            .filter(|base| !base.is_empty())
            .map(str::to_owned)
            .or_else(|| env.lookup(API_BASE_ENV).filter(|base| !base.is_empty()))
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
        Ok(Self {
            api_key,
            api_base,
            asset_id,
            unreachable_fallback: params.unreachable_fallback,
            timeout,
            request_timeout: timeout.unwrap_or(DEFAULT_REQUEST_TIMEOUT),
        })
    }
}

// LiteLLM falls through to get_secret_str for both names, and an empty value is no key at all.
fn resolved_api_key<E>(params: &RepelloAIParams, env: &E) -> Option<SecretString>
where
    E: EnvironmentLookup,
{
    let configured = params
        .api_key
        .as_ref()
        .filter(|key| !key.expose_secret().is_empty());
    if let Some(key) = configured {
        return Some(key.clone());
    }
    [API_KEY_ENV, FALLBACK_API_KEY_ENV]
        .into_iter()
        .find_map(|name| env.lookup(name).filter(|value| !value.is_empty()))
        .map(SecretString::from)
}

// LiteLLM's _event_hook_from_mode: a tagged Mode comes back untouched, and every name of a string
// or a list becomes a GuardrailEventHooks member, so an unknown hook fails the startup.
pub fn event_hook_from_mode(
    mode: &LitellmParamsMode,
) -> Result<LitellmParamsMode, RepelloLifecycleError> {
    match mode {
        LitellmParamsMode::Tagged(_) => Ok(mode.clone()),
        LitellmParamsMode::Many(names) => {
            names.iter().try_for_each(|name| check_event_hook(name))?;
            Ok(LitellmParamsMode::Many(names.clone()))
        }
        LitellmParamsMode::One(name) => {
            check_event_hook(name)?;
            Ok(LitellmParamsMode::One(name.clone()))
        }
    }
}

fn check_event_hook(name: &str) -> Result<(), RepelloLifecycleError> {
    let deserializer: StrDeserializer<'_, ValueError> = name.into_deserializer();
    GuardrailEventHooks::deserialize(deserializer)
        .map(|_hook| ())
        .map_err(|_unknown| RepelloLifecycleError::InvalidEventHook {
            hook: name.to_owned(),
        })
}
