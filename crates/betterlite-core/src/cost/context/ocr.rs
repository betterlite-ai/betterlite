use betterlite_models::ocr_pricing::{OcrPricing, layered_ocr_pricing};
use betterlite_models::price_lookup::PriceLookup;

use super::error::BillingContextError;
use super::pricing::{PricingQuery, deployment_info, price_record};

// Ports _ocr_model_info: top-level logging params overlay the extracted record, field by field, through B1678.
pub fn ocr_info(
    query: &PricingQuery<'_>,
    lookup: &dyn PriceLookup,
) -> Result<Option<OcrPricing>, BillingContextError> {
    let deployment = deployment_info(query, lookup)?;
    let overlay = query
        .logging_params
        .filter(|_| query.custom_pricing == Some(true));
    match (overlay, deployment) {
        (None, None) => Ok(None),
        (None, Some(record)) => layered_ocr_pricing(&[Some(&record)])
            .map(Some)
            .map_err(BillingContextError::from),
        (Some(params), extracted) => {
            let record = price_record(params);
            layered_ocr_pricing(&[Some(&record), extracted.as_ref()])
                .map(Some)
                .map_err(BillingContextError::from)
        }
    }
}
