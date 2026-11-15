use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::HookInitError;
use super::secret_lookup::SecretLookup;
use super::{ParamMap, read, required_name};

mod tests;

const VENDOR: &str = "Microsoft Agent 365";
const TENANT_ID_ENV: &str = "AGENT365_TENANT_ID";
const CLIENT_ID_ENV: &str = "AGENT365_CLIENT_ID";
const CLIENT_SECRET_ENV: &str = "AGENT365_CLIENT_SECRET";
const DEFAULT_REQUEST_TIMEOUT: f64 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoredOverride {
    ApiBase,
    ResourceAppId,
    AgentId,
}

impl IgnoredOverride {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ApiBase => "api_base",
            Self::ResourceAppId => "resource_app_id",
            Self::AgentId => "agent_id",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Agent365Args {
    pub guardrail_name: String,
    pub tenant_id: String,
    pub client_id: String,
    pub client_secret: SecretString,
    pub request_timeout: f64,
    pub unreachable_fallback: UnreachableFallback,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
}

pub fn initialize_agent_365(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
    lookup: &dyn SecretLookup,
) -> Result<(Agent365Args, Vec<IgnoredOverride>), HookInitError> {
    let (tenant_id, client_id, client_secret) = credentials(params, lookup)?;
    let name = required_name(guardrail_name, VENDOR)?;
    let args = Agent365Args {
        guardrail_name: name,
        tenant_id,
        client_id,
        client_secret,
        request_timeout: params
            .base
            .fields
            .timeout
            .unwrap_or(DEFAULT_REQUEST_TIMEOUT),
        unreachable_fallback: params.base.fields.unreachable_fallback,
        event_hook: params.mode.clone(),
        default_on: params.base.fields.default_on.unwrap_or_default(),
    };
    Ok((args, ignored_overrides(params)))
}

fn ignored_overrides(params: &LitellmParams) -> Vec<IgnoredOverride> {
    let extra = &params.base.extra.0;
    let mut overrides = Vec::new();
    if params.base.fields.api_base.is_some() {
        overrides.push(IgnoredOverride::ApiBase);
    }
    if extra
        .get("resource_app_id")
        .is_some_and(|value| *value != GuardrailValue::Null)
    {
        overrides.push(IgnoredOverride::ResourceAppId);
    }
    if extra
        .get("agent_id")
        .is_some_and(|value| *value != GuardrailValue::Null)
    {
        overrides.push(IgnoredOverride::AgentId);
    }
    overrides
}

fn credentials(
    params: &LitellmParams,
    lookup: &dyn SecretLookup,
) -> Result<(String, String, SecretString), HookInitError> {
    let extra = &params.base.extra.0;
    let tenant_id = text_or_secret(extra, "tenant_id", lookup, TENANT_ID_ENV)?.ok_or(
        HookInitError::Required {
            vendor: VENDOR,
            setting: "tenant_id",
        },
    )?;
    let client_id = text_or_secret(extra, "client_id", lookup, CLIENT_ID_ENV)?.ok_or(
        HookInitError::Required {
            vendor: VENDOR,
            setting: "client_id",
        },
    )?;
    let client_secret = read::<String>(extra, VENDOR, "client_secret")?
        .filter(|value| !value.is_empty())
        .map(SecretString::from)
        .or_else(|| {
            params
                .base
                .fields
                .api_key
                .clone()
                .filter(|key| !key.expose_secret().is_empty())
        })
        .or_else(|| {
            lookup
                .get_secret_str(CLIENT_SECRET_ENV)
                .filter(|secret| !secret.expose_secret().is_empty())
        })
        .ok_or(HookInitError::Agent365ClientSecretRequired)?;
    Ok((tenant_id, client_id, client_secret))
}

fn text_or_secret(
    map: &ParamMap,
    key: &str,
    lookup: &dyn SecretLookup,
    env: &str,
) -> Result<Option<String>, HookInitError> {
    Ok(read::<String>(map, VENDOR, key)?
        .filter(|value| !value.is_empty())
        .or_else(|| {
            lookup
                .get_secret_str(env)
                .map(|secret| secret.expose_secret().to_owned())
                .filter(|value| !value.is_empty())
        }))
}
