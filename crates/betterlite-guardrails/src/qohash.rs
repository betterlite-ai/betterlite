use std::collections::BTreeMap;
use std::future::Future;
use std::time::{Duration, TryFromFloatSecsError};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::config_models::GuardrailConfigModelIdentity;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::generic_guardrail_api::{
    DEFAULT_TIMEOUT, GenericGuardrailAPI, GenericGuardrailAPIError,
};
use crate::generic_guardrail_contracts::{
    GenericGuardrailAPISettings, GuardrailExtraValue, GuardrailInputType,
};
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use outcome::{blocked_failure, inputs_from_response};

pub mod init;
mod outcome;
mod request;
mod tests;

pub const GUARDRAIL_NAME: &str = "qostodian_nexus";
pub const DEFAULT_API_BASE: &str = "http://nexus:8800";
pub const API_BASE_VARIABLE: &str = "QOSTODIAN_NEXUS_API_BASE";
pub const GUARDRAIL_PATH: &str = "/beta/litellm_basic_guardrail_api";
pub const LITELLM_VERSION: &str = "1.105.1";
pub const NEXUS_HEADERS: [&str; 4] = [
    "x-qostodian-nexus-identifiers-trace",
    "x-qostodian-nexus-identifiers-source",
    "x-qostodian-nexus-identifiers-container",
    "x-qostodian-nexus-identifiers-identity",
];
pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 3] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::DuringCall,
    GuardrailEventHooks::PostCall,
];

#[derive(Debug, Error)]
pub enum QostodianNexusError {
    #[error("the Qostodian Nexus timeout is not a representable duration")]
    UnrepresentableTimeout {
        #[source]
        source: TryFromFloatSecsError,
    },
}

// LiteLLM's QostodianNexusConfigModel: the generic config model plus the Nexus api_base field
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QostodianNexusConfigModel {
    pub optional_params: Option<BTreeMap<String, GuardrailExtraValue>>,
    pub api_base: Option<String>,
}

impl GuardrailConfigModelIdentity for QostodianNexusConfigModel {
    fn ui_friendly_name() -> &'static str {
        "Qostodian Nexus"
    }
}

#[derive(Debug, Clone)]
pub struct QostodianNexusParams {
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub guardrail_name: Option<String>,
    pub extra_headers: Option<Vec<String>>,
    pub provider_params: BTreeMap<String, GuardrailExtraValue>,
    pub settings: GenericGuardrailAPISettings,
    pub client: reqwest::Client,
    pub timeout: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct QostodianNexus {
    pub generic: GenericGuardrailAPI,
    pub guardrail_name: String,
    pub extra_headers: Vec<String>,
}

impl QostodianNexus {
    pub fn new<L>(params: QostodianNexusParams, lookup: &L) -> Result<Self, QostodianNexusError>
    where
        L: EnvironmentLookup,
    {
        let QostodianNexusParams {
            api_key,
            api_base: configured_base,
            guardrail_name: configured_name,
            extra_headers: configured_headers,
            provider_params,
            settings,
            client,
            timeout: configured_timeout,
        } = params;
        let api_base = resolve_api_base(configured_base, lookup);
        let guardrail_name = configured_name.unwrap_or_else(|| GUARDRAIL_NAME.to_owned());
        let extra_headers = merge_headers(configured_headers);
        let timeout = match configured_timeout {
            Some(seconds) => Duration::try_from_secs_f64(seconds)
                .map_err(|source| QostodianNexusError::UnrepresentableTimeout { source })?,
            None => DEFAULT_TIMEOUT,
        };
        Ok(Self {
            generic: GenericGuardrailAPI {
                api_key,
                api_base,
                settings,
                provider_params,
                client,
                timeout,
            },
            guardrail_name,
            extra_headers,
        })
    }
}

impl GuardrailContract for QostodianNexus {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        let request = self.request_for(&inputs, request_data, input_type, logging_obj.as_deref());
        async move {
            match self.generic.apply_guardrail(request).await {
                Ok(response) => Ok(inputs_from_response(&inputs, response)),
                Err(GenericGuardrailAPIError::Blocked { reason }) => Err(blocked_failure(&reason)),
                Err(error) => Err(GuardrailFailure::Other(error.to_string())),
            }
        }
    }

    fn get_config_model(&self) -> Option<GuardrailValue> {
        let value = serde_json::to_value(QostodianNexusConfigModel::default()).ok()?;
        serde_json::from_value(value).ok()
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    }
}

fn resolve_api_base<L>(api_base: Option<String>, lookup: &L) -> String
where
    L: EnvironmentLookup,
{
    let supplied = api_base
        .filter(|base| !base.is_empty())
        .or_else(|| {
            lookup
                .lookup(API_BASE_VARIABLE)
                .filter(|base| !base.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned());
    if supplied.ends_with(GUARDRAIL_PATH) {
        supplied
    } else {
        format!("{}{GUARDRAIL_PATH}", supplied.trim_end_matches('/'))
    }
}

fn merge_headers(extra_headers: Option<Vec<String>>) -> Vec<String> {
    let mut merged: Vec<String> = NEXUS_HEADERS
        .iter()
        .map(|header| (*header).to_owned())
        .collect();
    for header in extra_headers.unwrap_or_default() {
        if !NEXUS_HEADERS.contains(&header.as_str()) {
            merged.push(header);
        }
    }
    merged
}
