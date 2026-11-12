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
    let billable = match record.fields.metadata.web_search_billing_unit {
        Some(WebSearchBillingUnit::PerPrompt) | None if requests > 0 => 1,
        Some(WebSearchBillingUnit::PerQuery | WebSearchBillingUnit::PerPrompt) | None => requests,
    };
    rate.checked_mul(Decimal::from(billable))
        .ok_or(GooglePricingError::Overflow { field: "search" })
}

pub fn maps_grounding_cost(
    usage: &GoogleUsage,
    record: &CatalogEntry,
) -> Result<Decimal, GooglePricingError> {
    let Some(requests) = usage
        .tokens
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.google_maps_grounding_requests)
        .filter(|count| *count > 0)
    else {
        return Ok(Decimal::ZERO);
    };
    let billing = record
        .fields
        .metadata
        .web_search_billing_unit
        .unwrap_or(WebSearchBillingUnit::PerPrompt);
    let default_rate = match billing {
        WebSearchBillingUnit::PerQuery => Decimal::new(14, 3),
        WebSearchBillingUnit::PerPrompt => Decimal::new(25, 3),
    };
    let rate = record
        .prices
        .values
        .other
        .google_maps_query
        .unwrap_or(default_rate);
    let billable = match billing {
        WebSearchBillingUnit::PerQuery => requests,
        WebSearchBillingUnit::PerPrompt => 1,
    };
    rate.checked_mul(Decimal::from(billable))
        .ok_or(GooglePricingError::Overflow { field: "maps" })
}
