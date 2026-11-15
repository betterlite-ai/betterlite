use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::Client;
use uuid::Uuid;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_initializers::extras;
use crate::hiddenlayer::evaluation::error::EvaluationError;
use crate::hiddenlayer::evaluation::{EvaluationArgs, config_model};
use crate::hiddenlayer::hiddenlayer_authentication::token::JwtTransport;
use crate::hiddenlayer::hiddenlayer_v1::HiddenlayerV1Guardrail;
use crate::hiddenlayer::hiddenlayer_v2::HiddenlayerV2Guardrail;

mod tests;

// HiddenlayerGuardrailConfigModel.version defaults to 2, and a null version reads as None
pub const DEFAULT_VERSION: i64 = 2;

pub enum HiddenlayerGuardrailInstance {
    V1(HiddenlayerV1Guardrail),
    V2(HiddenlayerV2Guardrail),
}

// LiteLLM's initialize_guardrail: a missing or first version runs v1, version 2 and up run v2
pub fn initialize_guardrail<L: EnvironmentLookup>(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
    environment: &L,
    client: Client,
    transport: Arc<dyn JwtTransport>,
    new_id: fn() -> Uuid,
) -> Result<HiddenlayerGuardrailInstance, EvaluationError> {
    let args = evaluation_args(params, guardrail_name)?;
    match extras::read(&params.base.extra, "version", Some(DEFAULT_VERSION))? {
        Some(version) if version >= DEFAULT_VERSION => Ok(HiddenlayerGuardrailInstance::V2(
            HiddenlayerV2Guardrail::new(&args, environment, client, transport, new_id)?,
        )),
        _ => Ok(HiddenlayerGuardrailInstance::V1(
            HiddenlayerV1Guardrail::new(&args, environment, client, transport)?,
        )),
    }
}

fn evaluation_args(
    params: &LitellmParams,
    guardrail_name: Option<&str>,
) -> Result<EvaluationArgs, EvaluationError> {
    let extra = &params.base.extra;
    Ok(EvaluationArgs {
        guardrail_name: Some(guardrail_name.unwrap_or_default().to_owned()),
        event_hook: params.mode.clone(),
        api_id: extras::read(extra, "api_id", None)?,
        api_key: params.base.fields.api_key.clone(),
        api_base: params.base.fields.api_base.clone(),
        auth_url: extras::read(extra, "auth_url", None)?,
        timeout: params.base.fields.timeout,
        default_on: params.base.fields.default_on.unwrap_or_default(),
    })
}

impl GuardrailContract for HiddenlayerGuardrailInstance {
    async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let evaluated = match self {
            Self::V1(guardrail) => {
                guardrail
                    .evaluate(inputs, request_data, input_type, logging_obj.as_deref())
                    .await
            }
            Self::V2(guardrail) => {
                guardrail
                    .evaluate(inputs, request_data, input_type, logging_obj.as_deref())
                    .await
            }
        };
        // LiteLLM lets the endpoint's refusal reach the caller; a provider or decode failure is a 500
        evaluated.map_err(|error| match error {
            EvaluationError::Blocked { status, detail } => {
                GuardrailFailure::HttpObject { status, detail }
            }
            other => GuardrailFailure::Http {
                status: 500,
                detail: other.to_string(),
            },
        })
    }

    fn get_config_model(&self) -> Option<GuardrailValue> {
        Some(config_model())
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        match self {
            Self::V1(guardrail) => guardrail.base.settings.supported_event_hooks.clone(),
            Self::V2(guardrail) => guardrail.base.settings.supported_event_hooks.clone(),
        }
    }
}
