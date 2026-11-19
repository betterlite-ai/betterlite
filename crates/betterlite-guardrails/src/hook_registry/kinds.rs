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

#[derive(Debug, Clone)]
pub struct AliceArgs {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub unreachable_fallback: UnreachableFallback,
    pub guardrail_name: String,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
}

#[must_use]
pub fn initialize_alice(params: &LitellmParams, guardrail_name: Option<&str>) -> AliceArgs {
    let fields = &params.base.fields;
    AliceArgs {
        api_key: fields.api_key.clone(),
        api_base: fields.api_base.clone(),
        unreachable_fallback: fields.unreachable_fallback,
        guardrail_name: guardrail_name.unwrap_or_default().to_owned(),
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
        timeout: fields.timeout,
    }
}

#[derive(Debug, Clone)]
pub struct CatoNetworksArgs {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub guardrail_name: String,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub inspect_embeddings: Option<bool>,
    pub ssl_verify: Option<VerifySetting>,
    pub timeout: Option<f64>,
}

pub fn initialize_cato_networks(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<CatoNetworksArgs, PortedInitError> {
    let fields = &params.base.fields;
    let extra = &params.base.extra.0;
    Ok(CatoNetworksArgs {
        api_base: fields.api_base.clone(),
        api_key: fields.api_key.clone(),
        guardrail_name: guardrail_name.unwrap_or_default().to_owned(),
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
        inspect_embeddings: fields.inspect_embeddings,
        ssl_verify: read("ssl_verify", extra.get("ssl_verify"))?,
        timeout: fields.timeout,
    })
}

#[derive(Debug, Clone)]
pub struct DynamoAiArgs {
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub guardrail_name: String,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
}

#[must_use]
pub fn initialize_dynamoai(params: &LitellmParams, guardrail_name: Option<&str>) -> DynamoAiArgs {
    let fields = &params.base.fields;
    DynamoAiArgs {
        api_base: fields.api_base.clone(),
        api_key: fields.api_key.clone(),
        guardrail_name: guardrail_name.unwrap_or_default().to_owned(),
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
        timeout: fields.timeout,
    }
}

#[derive(Debug, Clone)]
pub struct EnkryptaiArgs {
    pub guardrail_name: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub policy_name: Option<String>,
    pub deployment_name: Option<String>,
    pub detectors: Option<BTreeMap<String, GuardrailValue>>,
    pub block_on_violation: GuardrailFlag,
    pub event_hook: LitellmParamsMode,
    pub default_on: GuardrailFlag,
    pub timeout: Option<f64>,
}

pub fn initialize_enkryptai(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<EnkryptaiArgs, PortedInitError> {
    let fields = &params.base.fields;
    let extra = &params.base.extra.0;
    Ok(EnkryptaiArgs {
        guardrail_name: guardrail_name.unwrap_or_default().to_owned(),
        api_key: fields.api_key.clone(),
        api_base: fields.api_base.clone(),
        policy_name: read("policy_name", extra.get("policy_name"))?,
        deployment_name: read("deployment_name", extra.get("deployment_name"))?,
        detectors: read("detectors", extra.get("detectors"))?,
        block_on_violation: read("block_on_violation", extra.get("block_on_violation"))?
            .unwrap_or(true),
        event_hook: params.mode.clone(),
        default_on: fields.default_on.unwrap_or_default(),
        timeout: fields.timeout,
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
    let settings = HookSettings::from_params(params, guardrail_name);
    arms::build(kind, params, guardrail_name, &settings, ports)
}
