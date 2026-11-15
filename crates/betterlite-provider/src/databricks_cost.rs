use betterlite_models::cost_usage::CostUsage;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_cost::{
    GenericCostRequest, GenericCostResult, generic_cost_per_token,
};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

pub const PROVIDER: &str = "databricks";

const LEGACY_ENDPOINT_NAMES: [(&str, &str); 7] = [
    ("dbrx-instruct", "databricks-dbrx-instruct"),
    (
        "meta-llama-3.1-70b-instruct",
        "databricks-meta-llama-3-1-70b-instruct",
    ),
    (
        "meta-llama-3.1-405b-instruct",
        "databricks-meta-llama-3-1-405b-instruct",
    ),
    (
        "mixtral-8x7b-instruct-v0.1",
        "databricks-mixtral-8x7b-instruct",
    ),
    ("bge-large-en", "databricks-bge-large-en"),
    ("gte-large-en", "databricks-gte-large-en"),
    ("llama-2-70b-chat", "databricks-llama-2-70b-chat"),
];

pub struct CostRequest<'value> {
    pub model: &'value str,
    pub usage: &'value CostUsage,
    pub service_tier: Option<&'value str>,
    pub moment: DateTime<Utc>,
}

#[must_use]
pub fn registry_key(model: &str) -> String {
    let name = model.strip_prefix("databricks/").unwrap_or(model);
    // the table matches by prefix and replaces the whole name, so "dbrx-instruct-v2" prices as databricks-dbrx-instruct
    LEGACY_ENDPOINT_NAMES
        .iter()
        .find(|(prefix, _key)| name.starts_with(prefix))
        .map_or_else(|| name.to_owned(), |(_prefix, key)| (*key).to_owned())
}

pub fn cost_per_token(
    request: &CostRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<GenericCostResult, TokenCostError> {
    let model = registry_key(request.model);
    let generic = GenericCostRequest {
        model: &model,
        custom_llm_provider: Some(PROVIDER),
        usage: request.usage,
        prompt_extras: &PromptExtras::default(),
        supplied_pricing: None,
        service_tier: request.service_tier,
        data_residency: None,
        vertex_location: None,
        moment: request.moment,
    };
    generic_cost_per_token(&generic, lookup, database)
}

mod tests;
