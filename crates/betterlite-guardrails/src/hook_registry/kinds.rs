use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::{GuardrailFlag, GuardrailValue};
use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::tls::VerifySetting;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use thiserror::Error;

use super::arms;
use super::error::HookRegistryError;
use super::ports::HookPorts;
use super::spec::{HookSettings, HookSpec};
use crate::third_party_init_a::decode;

// The four folders whose class tasks build no initializer, so their LiteLLM initialize_guardrail is ported here
#[derive(Debug, Error)]
pub enum PortedInitError {
    #[error("the guardrail parameter {name} has an invalid value")]
    InvalidParam { name: String },
}

fn read<T>(name: &str, value: Option<&GuardrailValue>) -> Result<Option<T>, PortedInitError>
where
    T: DeserializeOwned,
{
    decode(name, value).map_err(|_source| PortedInitError::InvalidParam {
        name: name.to_owned(),
    })
}

// What all four classes take straight from LitellmParams before their own fields
#[derive(Debug, Clone)]
pub struct PortedBase {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub guardrail_name: String,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
}

fn ported_base(params: &LitellmParams, guardrail_name: Option<&str>) -> PortedBase {
    let fields = &params.base.fields;
    PortedBase {
        api_key: fields.api_key.clone(),
        api_base: fields.api_base.clone(),
        guardrail_name: guardrail_name.unwrap_or_default().to_owned(),
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
        timeout: fields.timeout,
    }
}

#[derive(Debug, Clone)]
pub struct AliceArgs {
    pub base: PortedBase,
    pub unreachable_fallback: UnreachableFallback,
}

#[must_use]
pub fn initialize_alice(params: &LitellmParams, guardrail_name: Option<&str>) -> AliceArgs {
    AliceArgs {
        base: ported_base(params, guardrail_name),
        unreachable_fallback: params.base.fields.unreachable_fallback,
    }
}

#[derive(Debug, Clone)]
pub struct CatoNetworksArgs {
    pub base: PortedBase,
    pub inspect_embeddings: Option<bool>,
    pub ssl_verify: Option<VerifySetting>,
}

pub fn initialize_cato_networks(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<CatoNetworksArgs, PortedInitError> {
    Ok(CatoNetworksArgs {
        base: ported_base(params, guardrail_name),
        inspect_embeddings: params.base.fields.inspect_embeddings,
        ssl_verify: read("ssl_verify", params.base.extra.0.get("ssl_verify"))?,
    })
}

#[derive(Debug, Clone)]
pub struct DynamoAiArgs {
    pub base: PortedBase,
}

#[must_use]
pub fn initialize_dynamoai(params: &LitellmParams, guardrail_name: Option<&str>) -> DynamoAiArgs {
    DynamoAiArgs {
        base: ported_base(params, guardrail_name),
    }
}

#[derive(Debug, Clone)]
pub struct EnkryptaiArgs {
    pub base: PortedBase,
    pub policy_name: Option<String>,
    pub deployment_name: Option<String>,
    pub detectors: Option<BTreeMap<String, GuardrailValue>>,
    pub block_on_violation: GuardrailFlag,
}

pub fn initialize_enkryptai(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<EnkryptaiArgs, PortedInitError> {
    let extra = &params.base.extra.0;
    let policy_name = read("policy_name", extra.get("policy_name"))?;
    let deployment_name = read("deployment_name", extra.get("deployment_name"))?;
    let detectors = read("detectors", extra.get("detectors"))?;
    let block_on_violation =
        read("block_on_violation", extra.get("block_on_violation"))?.unwrap_or(true);
    Ok(EnkryptaiArgs {
        base: ported_base(params, guardrail_name),
        policy_name,
        deployment_name,
        detectors,
        block_on_violation,
    })
}

// The table guardrail_registry.py builds by scanning guardrail_hooks, in folder order
#[must_use]
pub fn build_hook_spec(
    kind: &str,
    params: &LitellmParams,
    guardrail_name: &str,
    ports: &HookPorts,
) -> Option<Result<HookSpec, HookRegistryError>> {
    arms::build(
        kind,
        params,
        guardrail_name,
        &HookSettings::from_params(params, guardrail_name),
        ports,
    )
}
