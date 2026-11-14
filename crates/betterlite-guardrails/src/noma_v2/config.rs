use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::{ExposeSecret as _, SecretString};

use crate::noma_scan_policy::DEFAULT_API_BASE;

use super::SAAS_HOSTNAME;
use super::error::NomaV2Error;

pub const API_KEY_ENV: &str = "NOMA_API_KEY";
pub const API_BASE_ENV: &str = "NOMA_API_BASE";
pub const APPLICATION_ID_ENV: &str = "NOMA_APPLICATION_ID";
pub const GATEWAY_NAME_ENV: &str = "NOMA_GATEWAY_NAME";
pub const MONITOR_MODE_ENV: &str = "NOMA_MONITOR_MODE";
pub const BLOCK_FAILURES_ENV: &str = "NOMA_BLOCK_FAILURES";

// What LiteLLM's constructor receives: the configured values, before the environment fills the gaps
#[derive(Debug, Clone)]
pub struct NomaV2Params {
    pub guardrail_name: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub application_id: Option<String>,
    pub gateway_name: Option<String>,
    pub monitor_mode: Option<bool>,
    pub block_failures: Option<bool>,
    pub timeout: Option<f64>,
    pub mode: Option<LitellmParamsMode>,
    pub default_on: bool,
    pub strict_modes: bool,
}

#[derive(Debug, Clone)]
pub struct NomaV2Settings {
    pub api_key: Option<SecretString>,
    pub api_base: String,
    pub application_id: Option<String>,
    pub gateway_name: Option<String>,
    pub monitor_mode: bool,
    pub block_failures: bool,
    pub timeout: Option<Duration>,
}

impl NomaV2Params {
    pub fn from_litellm(
        params: &LitellmParams,
        guardrail_name: &str,
        strict_modes: bool,
    ) -> Result<Self, NomaV2Error> {
        let extra = &params.base.extra;
        Ok(Self {
            guardrail_name: Some(guardrail_name.to_owned()),
            api_key: params.base.fields.api_key.clone(),
            api_base: params.base.fields.api_base.clone(),
            application_id: text_param(extra, "application_id")?,
            gateway_name: text_param(extra, "gateway_name")?,
            monitor_mode: flag_param(extra, "monitor_mode")?,
            block_failures: flag_param(extra, "block_failures")?,
            timeout: params.base.fields.timeout,
            mode: Some(params.mode.clone()),
            default_on: params.base.fields.default_on.unwrap_or_default(),
            strict_modes,
        })
    }
}

impl NomaV2Settings {
    pub fn resolve<E>(params: &NomaV2Params, env: &E) -> Result<Self, NomaV2Error>
    where
        E: EnvironmentLookup,
    {
        let api_key = resolved_api_key(params, env);
        let api_base = resolved_api_base(params, env);
        let application_id =
            resolved_text(params.application_id.as_deref(), env, APPLICATION_ID_ENV);
        let gateway_name = resolved_text(params.gateway_name.as_deref(), env, GATEWAY_NAME_ENV);
        let monitor_mode = match params.monitor_mode {
            Some(monitor) => monitor,
            None => environment_flag(env, MONITOR_MODE_ENV, false)?,
        };
        let block_failures = match params.block_failures {
            Some(block) => block,
            None => environment_flag(env, BLOCK_FAILURES_ENV, true)?,
        };
        let timeout = params.timeout.map(seconds_timeout).transpose()?;
        if saas_api_base(&api_base) && api_key.is_none() {
            return Err(NomaV2Error::MissingApiKey);
        }
        Ok(Self {
            api_key,
            api_base,
            application_id,
            gateway_name,
            monitor_mode,
            block_failures,
            timeout,
        })
    }
}

fn resolved_api_key<E>(params: &NomaV2Params, env: &E) -> Option<SecretString>
where
    E: EnvironmentLookup,
{
    let configured = params
        .api_key
        .as_ref()
        .and_then(|key| trimmed(key.expose_secret()));
    let text = configured.or_else(|| env.lookup(API_KEY_ENV).and_then(|text| trimmed(&text)))?;
    Some(SecretString::from(text))
}

fn resolved_api_base<E>(params: &NomaV2Params, env: &E) -> String
where
    E: EnvironmentLookup,
{
    let base = match params.api_base.as_deref() {
        Some(base) => base.to_owned(),
        None => env
            .lookup(API_BASE_ENV)
            .filter(|base| !base.is_empty())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_owned()),
    };
    base.trim_end_matches('/').to_owned()
}

fn resolved_text<E>(configured: Option<&str>, env: &E, name: &str) -> Option<String>
where
    E: EnvironmentLookup,
{
    configured
        .and_then(trimmed)
        .or_else(|| env.lookup(name).and_then(|text| trimmed(&text)))
}

fn text_param(extra: &OpaqueParams, field: &'static str) -> Result<Option<String>, NomaV2Error> {
    match extra.0.get(field) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(text)) => Ok(Some(text.clone())),
        Some(_wrong) => Err(NomaV2Error::Param { field }),
    }
}

fn flag_param(extra: &OpaqueParams, field: &'static str) -> Result<Option<bool>, NomaV2Error> {
    match extra.0.get(field) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Boolean(flag)) => Ok(Some(*flag)),
        Some(_wrong) => Err(NomaV2Error::Param { field }),
    }
}

fn environment_flag<E>(env: &E, name: &str, default: bool) -> Result<bool, NomaV2Error>
where
    E: EnvironmentLookup,
{
    let Some(text) = env.lookup(name) else {
        return Ok(default);
    };
    Ok(betterlite_env::flag(name, &text)?.unwrap_or(default))
}

fn seconds_timeout(seconds: f64) -> Result<Duration, NomaV2Error> {
    Duration::try_from_secs_f64(seconds).map_err(|source| NomaV2Error::Timeout { seconds, source })
}

fn saas_api_base(api_base: &str) -> bool {
    let Ok(parsed) = url::Url::parse(api_base) else {
        return false;
    };
    parsed.host_str() == Some(SAAS_HOSTNAME)
}

// LiteLLM's _get_non_empty_str: a value that is only whitespace counts as absent
pub(super) fn trimmed(text: &str) -> Option<String> {
    let stripped = text.trim();
    if stripped.is_empty() {
        return None;
    }
    Some(stripped.to_owned())
}
