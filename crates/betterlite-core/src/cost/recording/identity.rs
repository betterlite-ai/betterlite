use std::str::FromStr as _;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_logging::logging_metadata::router::RouterModelError;
use betterlite_logging::logging_privacy::internal::{Origin, call_category, is_unbilled};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::{MantleRegionResolver, PortFailure, VertexRegionNormalizer};
use crate::cost::context::error::BillingContextError;

pub(super) mod metadata;

const BACKGROUND_COST_POLL_ORIGIN: &str = "background_response_cost_poll";
const SPEECH_CALL_TYPES: [&str; 2] = ["speech", "aspeech"];
const VERTEX_PROVIDER_PREFIX: &str = "vertex_ai";
const MANTLE_PROVIDER: &str = "bedrock_mantle";

#[derive(Debug, thiserror::Error)]
pub(super) enum PreparationError {
    #[error("the {field} value must be an object or a falsy value")]
    Metadata { field: &'static str },
    #[error("the base_model value must be a string")]
    BaseModel,
    #[error("the {field} value must be a string")]
    Field { field: &'static str },
    #[error("the router model id could not be read: {0}")]
    Router(#[source] RouterModelError),
    #[error("the deployment model info could not be read: {0}")]
    Pricing(#[from] BillingContextError),
    #[error("the bedrock region could not be resolved: {0}")]
    Mantle(#[source] PortFailure),
}

// Every value response_cost_calculator reads, copied so the call log can be mutated afterwards.
pub(super) struct BillingIdentity {
    pub(super) model: String,
    pub(super) cache_hit: Option<bool>,
    pub(super) custom_llm_provider: Option<String>,
    pub(super) base_model: Option<String>,
    pub(super) custom_pricing: bool,
    pub(super) router_model_id: Option<String>,
    pub(super) data_residency: Option<String>,
    pub(super) vertex_location: Option<String>,
    pub(super) region_name: Option<String>,
    pub(super) prompt: String,
}

pub(super) fn custom_llm_provider(log: &LoggingState) -> Result<Option<String>, PreparationError> {
    match log.model_call_details.get("custom_llm_provider") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(PreparationError::Field {
                field: "custom_llm_provider",
            })
        }
    }
}

pub(super) fn data_residency(log: &LoggingState) -> Result<Option<String>, PreparationError> {
    match log.litellm_params.get("data_residency") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(PreparationError::Field {
                field: "data_residency",
            })
        }
    }
}

// _prompt_for_cost_calculation: only text-to-speech prices its input string directly.
pub(super) fn prompt(log: &LoggingState) -> String {
    if !SPEECH_CALL_TYPES.contains(&log.call_type.as_str()) {
        return String::new();
    }
    match log.model_call_details.get("input") {
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => String::new(),
    }
}

// _resolve_vertex_location_for_cost: explicit parameters, then the configured default, then select_region.
pub(super) fn vertex_location(
    log: &LoggingState,
    model: &str,
    settings: &VertexSettings,
    regions: &dyn VertexRegionNormalizer,
) -> Result<Option<String>, PreparationError> {
    let provider = match log.model_call_details.get("custom_llm_provider") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Text(text)) => text,
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(PreparationError::Field {
                field: "custom_llm_provider",
            });
        }
    };
    if !provider.starts_with(VERTEX_PROVIDER_PREFIX) {
        return Ok(None);
    }
    let configured = explicit_location(&log.optional_params)
        .or_else(|| explicit_location(&log.litellm_params))
        .or(settings.location.as_deref());
    Ok(Some(regions.region(model, configured)))
}

fn explicit_location(params: &IndexMap<String, Json>) -> Option<&str> {
    for field in ["vertex_location", "vertex_ai_location"] {
        if let Some(Json::Text(text)) = params.get(field)
            && !text.is_empty()
        {
            return Some(text);
        }
    }
    None
}

// _resolve_mantle_region_for_cost: only bedrock_mantle resolves a region, from the logged params.
pub(super) fn mantle_region(
    log: &LoggingState,
    mantle: &dyn MantleRegionResolver,
) -> Result<Option<String>, PreparationError> {
    match log.model_call_details.get("custom_llm_provider") {
        Some(Json::Text(provider)) if provider == MANTLE_PROVIDER => {}
        Some(Json::Text(_)) | None | Some(Json::Null) => return Ok(None),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => return Ok(None),
    }
    let params = match log.model_call_details.get("litellm_params") {
        Some(Json::Object(fields)) => fields,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => {
            return mantle
                .region(&IndexMap::new())
                .map(Some)
                .map_err(PreparationError::Mantle);
        }
    };
    mantle
        .region(params)
        .map(Some)
        .map_err(PreparationError::Mantle)
}

// A stored cache flag the caller did not pass through: an absent key reads False, an explicit null None.
pub(super) fn stored_cache_hit(log: &LoggingState) -> Option<bool> {
    match log.model_call_details.get("cache_hit") {
        None => Some(false),
        Some(Json::Bool(flag)) => Some(*flag),
        Some(Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            None
        }
    }
}

// additional_response_cost adds only a positive number; Python's bool counts as 1 or 0.
pub(super) fn additional_cost(log: &LoggingState) -> Option<Decimal> {
    match log.model_call_details.get("additional_response_cost") {
        Some(Json::Number(number)) => Decimal::from_str(&number.to_string()).ok(),
        Some(Json::Bool(true)) => Some(Decimal::ONE),
        Some(Json::Bool(false)) => Some(Decimal::ZERO),
        Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) | None => None,
    }
}

pub(super) fn total_with_additional(amount: Decimal, additional: Option<Decimal>) -> Decimal {
    match additional {
        Some(extra) if extra > Decimal::ZERO => match amount.checked_add(extra) {
            Some(total) => total,
            None => amount,
        },
        Some(_) | None => amount,
    }
}

pub(super) fn amount(value: &Json) -> Option<Decimal> {
    match value {
        Json::Number(number) => Decimal::from_str(&number.to_string()).ok(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// is_unbilled_non_inference_call: only management reads are free, minus background responses and the poller.
pub(super) fn unbilled(log: &LoggingState, background: bool) -> bool {
    is_unbilled(
        call_category(Some(&log.call_type)),
        cost_poll_origin(&log.litellm_params),
        background,
    )
}

// The origin is looked up in metadata first, then litellm_metadata, as LiteLLM's merge does.
fn cost_poll_origin(litellm_params: &IndexMap<String, Json>) -> Option<Origin> {
    let found =
        ["metadata", "litellm_metadata"]
            .iter()
            .find_map(|name| match litellm_params.get(*name) {
                Some(Json::Object(fields)) => fields.get("internal_call_origin"),
                Some(
                    Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_),
                )
                | None => None,
            });
    match found {
        Some(Json::Text(origin)) if origin == BACKGROUND_COST_POLL_ORIGIN => {
            Some(Origin::BackgroundResponseCostPoll)
        }
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}
