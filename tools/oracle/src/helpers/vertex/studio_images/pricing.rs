use betterlite_models::image_cost::error::ImageCostError;
use betterlite_models::image_cost::pricing::resolve_image_model_info;
use betterlite_models::image_cost::usage::{
    ImageUsageCostRequest, ImageUsageDetails, image_usage_cost,
};
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_vertex::vertex_image_policy::cost::ImagePricing;
use chrono::DateTime;
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

// Gemini grounding bills one default request fee per grounded prompt; no case sets a billing unit
const SEARCH_FEE: Decimal = Decimal::from_parts(35, 0, 0, false, 3);

// The case's catalog entry answers every effective lookup, standing in for litellm.model_cost.
pub(super) struct CatalogPricing<'catalog> {
    pub(super) catalog: &'catalog PriceRecord,
}

impl PriceLookup for CatalogPricing<'_> {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        Some(self.catalog.clone())
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

struct UsageBilling<'billing> {
    model: &'billing str,
    provider: &'billing str,
    usage: &'billing ImageUsageDetails,
    model_info: &'billing PriceRecord,
}

impl ImagePricing for CatalogPricing<'_> {
    fn model_info(
        &self,
        model: &str,
        provider: &str,
        supplied: Option<&PriceRecord>,
    ) -> Result<PriceRecord, ImageCostError> {
        resolve_image_model_info(model, Some(provider), supplied, self)
    }

    fn web_search_cost(
        &self,
        _provider: &str,
        requests: Option<i64>,
        _model_info: &PriceRecord,
    ) -> Decimal {
        match requests {
            Some(count) if count > 0 => SEARCH_FEE,
            Some(_) | None => Decimal::ZERO,
        }
    }

    fn usage_cost(
        &self,
        model: &str,
        provider: &str,
        usage: &ImageUsageDetails,
        model_info: &PriceRecord,
    ) -> Result<Option<Decimal>, ImageCostError> {
        billed_usage(
            self,
            UsageBilling {
                model,
                provider,
                usage,
                model_info,
            },
        )
    }
}

fn billed_usage(
    lookup: &dyn PriceLookup,
    billing: UsageBilling<'_>,
) -> Result<Option<Decimal>, ImageCostError> {
    let request = ImageUsageCostRequest {
        model: billing.model,
        custom_llm_provider: Some(billing.provider),
        usage: billing.usage,
        model_info: Some(billing.model_info),
        moment: DateTime::UNIX_EPOCH,
    };
    image_usage_cost(&request, lookup, &TimeZoneDatabase::none())
}
