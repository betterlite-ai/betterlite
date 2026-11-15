pub mod error;
mod tests;

use betterlite_models::cost_usage::CostUsage;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_cost::{GenericCostRequest, generic_cost_per_token};
use betterlite_models::token_rates::ModelInfo;
use chrono::{DateTime, Utc};
use fancy_regex::Regex;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use error::{FireworksCostError, LookupFailed};

pub const FIREWORKS_AI_PROVIDER: &str = "fireworks_ai";

const MOE_PATTERN: &str = r"(\d+)x(\d+)b";
const SIZE_PATTERN: &str = r"(\d+)b";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FireworksPricingTiers {
    pub up_to_4b: u128,
    pub up_to_16b: u128,
    pub moe_up_to_56b: u128,
    pub moe_up_to_176b: u128,
}
impl Default for FireworksPricingTiers {
    fn default() -> Self {
        Self {
            up_to_4b: 4,
            up_to_16b: 16,
            moe_up_to_56b: 56,
            moe_up_to_176b: 176,
        }
    }
}
pub trait ModelInfoLookup {
    fn get(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Result<ModelInfo, LookupFailed>;
}
fn moe_billions(name: &str) -> Option<u128> {
    let pattern = Regex::new(MOE_PATTERN).ok()?;
    let captures = pattern.captures(name).ok()??;
    let first = captures
        .get(1)?
        .as_str()
        .parse::<u128>()
        .unwrap_or(u128::MAX);
    let second = captures
        .get(2)?
        .as_str()
        .parse::<u128>()
        .unwrap_or(u128::MAX);
    Some(first.saturating_mul(second))
}
fn standard_billions(name: &str) -> Option<u128> {
    let pattern = Regex::new(SIZE_PATTERN).ok()?;
    let captures = pattern.captures(name).ok()??;
    let digits = captures.get(1)?.as_str();
    Some(digits.parse::<u128>().unwrap_or(u128::MAX))
}
#[must_use]
pub fn get_base_model_for_pricing(model: &str, tiers: &FireworksPricingTiers) -> &'static str {
    let name = model.to_lowercase();
    if let Some(total) = moe_billions(&name) {
        if total <= tiers.moe_up_to_56b {
            return "fireworks-ai-moe-up-to-56b";
        }
        if total <= tiers.moe_up_to_176b {
            return "fireworks-ai-56b-to-176b";
        }
    }
    if let Some(params) = standard_billions(&name) {
        if params <= tiers.up_to_4b {
            return "fireworks-ai-up-to-4b";
        }
        if params <= tiers.up_to_16b {
            return "fireworks-ai-4.1b-to-16b";
        }
        return "fireworks-ai-above-16b";
    }
    "fireworks-ai-default"
}
fn resolve_model_info(
    model: &str,
    tiers: &FireworksPricingTiers,
    lookup: &dyn ModelInfoLookup,
) -> Result<ModelInfo, FireworksCostError> {
    if let Ok(record) = lookup.get(model, Some(FIREWORKS_AI_PROVIDER)) {
        return Ok(record);
    }
    let stripped = model.strip_prefix("fireworks_ai/").unwrap_or(model);
    if let Ok(record) = lookup.get(stripped, None) {
        return Ok(record);
    }
    let base = get_base_model_for_pricing(model, tiers);
    lookup
        .get(base, Some(FIREWORKS_AI_PROVIDER))
        .map_err(|source| FireworksCostError::Lookup {
            model: model.to_owned(),
            source,
        })
}
// The supplied record is always the pricing source, so the catalog lookup never runs.
struct SuppliedPricingOnly;

impl PriceLookup for SuppliedPricingOnly {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}
pub fn cost_per_token(
    model: &str,
    usage: &CostUsage,
    instant: DateTime<Utc>,
    tiers: &FireworksPricingTiers,
    lookup: &dyn ModelInfoLookup,
) -> Result<(Decimal, Decimal), FireworksCostError> {
    let record = resolve_model_info(model, tiers, lookup)?;
    let Some(Scalar::Text(provider)) = record.get("litellm_provider") else {
        return Err(FireworksCostError::Provider {
            model: model.to_owned(),
        });
    };
    let billing_provider = provider.clone();
    let pricing = PricedModel {
        record,
        tiered_pricing: Vec::new(),
        off_peak_pricing: None,
        off_peak_cache_read_present: false,
    };
    let prompt_extras = PromptExtras::default();
    let request = GenericCostRequest {
        model,
        custom_llm_provider: Some(billing_provider.as_str()),
        usage,
        prompt_extras: &prompt_extras,
        supplied_pricing: Some(&pricing),
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        moment: instant,
    };
    let result = generic_cost_per_token(&request, &SuppliedPricingOnly, &TimeZoneDatabase::none())?;
    Ok((result.prompt_cost, result.completion_cost))
}
