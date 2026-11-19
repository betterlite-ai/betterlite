use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_logging::logging_metadata::params::LitellmParams;
use betterlite_logging::logging_metadata::router::get_router_model_id;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::pricing::{CUSTOM_PRICING_KEYS, takes_custom_pricing};
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::ptu_pricing::PtuModelInfo;
use betterlite_models::ptu_pricing::spillover::is_spilled_over_ptu_request;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::super::CostRecorder;
use super::super::view;
use super::PreparationError;
use crate::cost::context::pricing::{PricingQuery, deployment_info};
use crate::cost::prepare::ResponseView;

impl<Inference> CostRecorder<'_, Inference> {
    // Ports _custom_pricing_for: a spilled-over PTU request never reads custom prices.
    pub(crate) fn custom_pricing(
        &self,
        log: &LoggingState,
        view: &ResponseView<'_>,
    ) -> Result<bool, PreparationError> {
        if self.ptu_cost_attribution {
            let record = deployment_info(
                &PricingQuery {
                    custom_pricing: Some(true),
                    router_model_id: None,
                    logging_params: Some(&log.litellm_params),
                },
                self.lookup,
            )?;
            let empty = PriceRecord::new();
            let model_info = PtuModelInfo::from(record.as_ref().unwrap_or(&empty));
            let response_headers = string_map(log.model_call_details.get("response_headers"));
            let additional_headers = view::hidden_params(view)
                .and_then(|hidden| string_map(hidden.get("additional_headers")));
            if is_spilled_over_ptu_request(
                &model_info,
                response_headers.as_ref(),
                additional_headers.as_ref(),
                true,
            ) {
                return Ok(false);
            }
        }
        uses_custom_pricing(&log.litellm_params)
    }
}

// use_custom_pricing_for_model: the top-level pricing fields first, then each deployment model_info.
pub(crate) fn uses_custom_pricing(
    litellm_params: &IndexMap<String, Json>,
) -> Result<bool, PreparationError> {
    if takes_custom_pricing(litellm_params) {
        return Ok(true);
    }
    for field in ["metadata", "litellm_metadata"] {
        let Some(metadata) = optional_object(litellm_params.get(field), field)? else {
            continue;
        };
        let Some(Json::Object(model_info)) = metadata.get("model_info") else {
            continue;
        };
        let priced = CUSTOM_PRICING_KEYS.iter().any(|key| {
            model_info
                .get(*key)
                .is_some_and(|value| *value != Json::Null)
        });
        if priced {
            return Ok(true);
        }
        let off_peak = model_info
            .get("off_peak_pricing")
            .is_some_and(|value| *value != Json::Null);
        if off_peak {
            return Ok(true);
        }
    }
    Ok(false)
}

// _get_base_model_from_metadata: a non-null top-level value wins, then metadata, then litellm_metadata.
pub(crate) fn base_model(log: &LoggingState) -> Result<Option<String>, PreparationError> {
    match log.litellm_params.get("base_model") {
        None | Some(Json::Null) => {}
        Some(Json::Text(text)) => return Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(PreparationError::BaseModel);
        }
    }
    for field in ["metadata", "litellm_metadata"] {
        if let Some(found) = metadata_base_model(log.litellm_params.get(field), field)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn metadata_base_model(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<String>, PreparationError> {
    let Some(metadata) = optional_object(value, field)? else {
        return Ok(None);
    };
    let Some(model_info) = metadata.get("model_info") else {
        return Ok(None);
    };
    let Some(model_info) = present_object(model_info, field)? else {
        return Ok(None);
    };
    match model_info.get("base_model") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(PreparationError::BaseModel)
        }
    }
}

// get_router_model_id reads litellm_metadata before metadata, both taken from litellm_params.
pub(crate) fn router_model_id(log: &LoggingState) -> Result<Option<String>, PreparationError> {
    for field in ["litellm_metadata", "metadata"] {
        optional_object(log.litellm_params.get(field), field)?;
    }
    get_router_model_id(&params_view(&log.litellm_params)).map_err(PreparationError::Router)
}

pub(crate) fn params_view(litellm_params: &IndexMap<String, Json>) -> LitellmParams {
    LitellmParams {
        metadata: object_field(litellm_params.get("metadata")),
        litellm_metadata: object_field(litellm_params.get("litellm_metadata")),
        litellm_session_id: None,
        litellm_trace_id: None,
        prompt_id: None,
        prompt_variables: None,
        langfuse_masking_function: None,
    }
}

fn object_field(value: Option<&Json>) -> Option<IndexMap<String, Json>> {
    match value {
        Some(Json::Object(fields)) => Some(fields.clone()),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

fn optional_object<'value>(
    value: Option<&'value Json>,
    field: &'static str,
) -> Result<Option<&'value IndexMap<String, Json>>, PreparationError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(Json::Bool(false)) => Ok(None),
        Some(Json::Number(number)) if number_is_zero(number) => Ok(None),
        Some(Json::Text(text)) if text.is_empty() => Ok(None),
        Some(Json::List(items)) if items.is_empty() => Ok(None),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(PreparationError::Metadata { field })
        }
    }
}

fn present_object<'value>(
    value: &'value Json,
    field: &'static str,
) -> Result<Option<&'value IndexMap<String, Json>>, PreparationError> {
    match value {
        Json::Object(fields) if fields.is_empty() => Ok(None),
        Json::Object(fields) => Ok(Some(fields)),
        Json::Null => Ok(None),
        Json::Bool(false) => Ok(None),
        Json::Number(number) if number_is_zero(number) => Ok(None),
        Json::Text(text) if text.is_empty() => Ok(None),
        Json::List(items) if items.is_empty() => Ok(None),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(PreparationError::Metadata { field })
        }
    }
}

fn number_is_zero(number: &serde_json::Number) -> bool {
    Decimal::from_str(&number.to_string()).is_ok_and(|value| value.is_zero())
}

pub(crate) fn string_map(value: Option<&Json>) -> Option<BTreeMap<String, String>> {
    let Json::Object(fields) = value? else {
        return None;
    };
    Some(
        fields
            .iter()
            .filter_map(|(name, value)| match value {
                Json::Text(text) => Some((name.clone(), text.clone())),
                Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    None
                }
            })
            .collect(),
    )
}
