use betterlite_models::catalog::CatalogEntry;
use betterlite_models::metadata::vendor::WebSearchBillingUnit;
use rust_decimal::Decimal;

use super::error::GooglePricingError;
use super::usage::GoogleUsage;

pub fn cost_per_web_search_request(
    usage: &GoogleUsage,
    record: &CatalogEntry,
) -> Result<Decimal, GooglePricingError> {
    let from_prompt_details = usage
        .tokens
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.web_search_requests);
    // the source's `a or b or 0` chain lets a zero prompt-detail count fall through to the server tool use count
    let requests = from_prompt_details
        .filter(|count| *count != 0)
        .or_else(|| {
            usage
                .server_tool_use
                .web_search_requests
                .filter(|count| *count != 0)
        })
        .unwrap_or_default();
    let rate = record
        .prices
        .search_context
        .as_ref()
        .and_then(|pricing| pricing.search_context_size_medium)
        .unwrap_or_else(|| Decimal::new(35, 3));
    let per_prompt = !matches!(
        record.fields.metadata.web_search_billing_unit,
        Some(WebSearchBillingUnit::PerQuery)
    );
    let billable = if per_prompt && requests > 0 {
        1
    } else {
        requests
    };
    rate_charge(rate, billable, "search")
}

pub fn maps_grounding_cost(
    usage: &GoogleUsage,
    entry: &CatalogEntry,
) -> Result<Decimal, GooglePricingError> {
    let Some(queries) = usage
        .tokens
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.google_maps_grounding_requests)
        .filter(|count| *count > 0)
    else {
        return Ok(Decimal::ZERO);
    };
    let billing_unit = entry
        .fields
        .metadata
        .web_search_billing_unit
        .unwrap_or(WebSearchBillingUnit::PerPrompt);
    let fallback_price = match billing_unit {
        WebSearchBillingUnit::PerQuery => Decimal::new(14, 3),
        WebSearchBillingUnit::PerPrompt => Decimal::new(25, 3),
    };
    let price = entry
        .prices
        .values
        .other
        .google_maps_query
        .unwrap_or(fallback_price);
    let billed_queries = match billing_unit {
        WebSearchBillingUnit::PerQuery => queries,
        WebSearchBillingUnit::PerPrompt => 1,
    };
    rate_charge(price, billed_queries, "maps")
}

fn rate_charge(
    rate: Decimal,
    count: i64,
    field: &'static str,
) -> Result<Decimal, GooglePricingError> {
    rate.checked_mul(Decimal::from(count))
        .ok_or(GooglePricingError::Overflow { field })
}
