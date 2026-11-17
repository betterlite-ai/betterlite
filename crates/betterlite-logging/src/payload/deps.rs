use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_models::cost_breakdown::CostBreakdown;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::cold_storage_key::ColdStorageLocator;
use crate::fields::usage::{Usage, UsageError};
use crate::logging_metadata::params::MetadataMap;

pub const MODEL_MAP_KEY: &str = "model_map_key";
pub const MODEL_MAP_VALUE: &str = "model_map_value";

// The payload's cache-hit id suffix needs the wall clock, which a test replaces with a fixed reading
pub trait PayloadClock: Send + Sync {
    fn now_epoch_seconds(&self) -> f64;
}

#[derive(Debug, Default)]
pub struct SystemPayloadClock;

impl PayloadClock for SystemPayloadClock {
    fn now_epoch_seconds(&self) -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |elapsed| elapsed.as_secs_f64())
    }
}

pub struct ModelCostRequest<'request> {
    pub model_call_details: &'request IndexMap<String, Json>,
    pub init_response_obj: &'request Json,
    pub api_base: Option<&'request str>,
    pub custom_llm_provider: Option<&'request str>,
}

// get_model_cost_information is B068's; the payload only carries its model_map_information result
pub trait ModelCostInformation: Send + Sync {
    fn model_map_information(&self, request: &ModelCostRequest<'_>) -> IndexMap<String, Json>;
}

#[derive(Debug, Default)]
pub struct AbsentModelCostInformation;

impl ModelCostInformation for AbsentModelCostInformation {
    fn model_map_information(&self, _request: &ModelCostRequest<'_>) -> IndexMap<String, Json> {
        IndexMap::from([
            (MODEL_MAP_KEY.to_owned(), Json::Text(String::new())),
            (MODEL_MAP_VALUE.to_owned(), Json::Null),
        ])
    }
}

// _response_cost_calculator's cached answer is B068's; None leaves saved_cache_cost at zero
pub trait SavedCacheCost: Send + Sync {
    fn saved_cache_cost(&self, init_response_obj: &Json) -> Option<Decimal>;
}

#[derive(Debug, Default)]
pub struct AbsentSavedCacheCost;

impl SavedCacheCost for AbsentSavedCacheCost {
    fn saved_cache_cost(&self, _init_response_obj: &Json) -> Option<Decimal> {
        None
    }
}

// The auth layer stamps access groups (B052); an absent source logs no groups
pub trait RequestAccessGroups: Send + Sync {
    fn access_groups(&self, litellm_params: &IndexMap<String, Json>) -> Option<Vec<String>>;
}

#[derive(Debug, Default)]
pub struct AbsentRequestAccessGroups;

impl RequestAccessGroups for AbsentRequestAccessGroups {
    fn access_groups(&self, _litellm_params: &IndexMap<String, Json>) -> Option<Vec<String>> {
        None
    }
}

pub struct SavingsRequest<'request> {
    pub metadata: &'request MetadataMap,
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub model_id: &'request str,
    pub usage_object: &'request Json,
    pub cost_breakdown: Option<&'request CostBreakdown>,
}

// _autorouter_savings_for_payload is B021's; its baseline belongs to the same track
pub trait AutorouterAudit: Send + Sync {
    fn savings(&self, request: &SavingsRequest<'_>) -> Option<Decimal>;
    fn baseline_observation(&self) -> Option<String>;
    fn baseline_context_active(&self) -> bool;
}

#[derive(Debug, Default)]
pub struct AbsentAutorouterAudit;

impl AutorouterAudit for AbsentAutorouterAudit {
    fn savings(&self, _request: &SavingsRequest<'_>) -> Option<Decimal> {
        None
    }

    fn baseline_observation(&self) -> Option<String> {
        None
    }

    fn baseline_context_active(&self) -> bool {
        false
    }
}

pub type OwnedResponsesConverter =
    Arc<dyn Fn(&IndexMap<String, Json>) -> Result<Usage, UsageError> + Send + Sync>;

// The Responses API usage conversion is B906's; without it a Responses usage object cannot be read
fn unconverted_responses_usage(_mapping: &IndexMap<String, Json>) -> Result<Usage, UsageError> {
    Err(UsageError::ProviderConversion {
        reason: "the Responses API usage conversion is not configured".to_owned(),
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagConfig {
    pub disable_user_agent_tags: bool,
    pub extra_spend_tag_headers: Vec<String>,
}

pub struct PayloadSources {
    pub model_cost: Arc<dyn ModelCostInformation>,
    pub saved_cache_cost: Arc<dyn SavedCacheCost>,
    pub access_groups: Arc<dyn RequestAccessGroups>,
    pub autorouter: Arc<dyn AutorouterAudit>,
    pub cold_storage: Option<Arc<dyn ColdStorageLocator>>,
    pub clock: Arc<dyn PayloadClock>,
    pub responses_converter: OwnedResponsesConverter,
}

impl Default for PayloadSources {
    fn default() -> Self {
        Self {
            model_cost: Arc::new(AbsentModelCostInformation),
            saved_cache_cost: Arc::new(AbsentSavedCacheCost),
            access_groups: Arc::new(AbsentRequestAccessGroups),
            autorouter: Arc::new(AbsentAutorouterAudit),
            cold_storage: None,
            clock: Arc::new(SystemPayloadClock),
            responses_converter: Arc::new(unconverted_responses_usage),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PayloadSettings {
    pub tag_config: TagConfig,
    pub request_correlation_in_logs: bool,
    pub log_client_error_tracebacks: bool,
}
