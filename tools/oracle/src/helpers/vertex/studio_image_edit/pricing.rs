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

// A copy of image_policy's private double: its module is outside this task's scope.
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

impl ImagePricing for CatalogPricing<'_> {
    fn model_info(
        &self,
        model: &str,
        provider: &str,
        supplied: Option<&PriceRecord>,
    ) -> Result<PriceRecord, ImageCostError> {
        resolve_image_model_info(model, Some(provider), supplied, self)
    }

    // the edit adapter never carries a search count, so the grounded-request fee never applies
    fn web_search_cost(
        &self,
        _provider: &str,
        _web_search_requests: Option<i64>,
        _model_info: &PriceRecord,
    ) -> Decimal {
        Decimal::ZERO
    }

    fn usage_cost(
        &self,
        model: &str,
        provider: &str,
        usage: &ImageUsageDetails,
        model_info: &PriceRecord,
    ) -> Result<Option<Decimal>, ImageCostError> {
        let request = ImageUsageCostRequest {
            model,
            custom_llm_provider: Some(provider),
            usage,
            model_info: Some(model_info),
            moment: DateTime::UNIX_EPOCH,
        };
        image_usage_cost(&request, self, &TimeZoneDatabase::none())
    }
}
