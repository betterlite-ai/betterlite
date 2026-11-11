use betterlite_logging::fields::timestamps::TimestampError;
use betterlite_models::ocr_pricing::OcrCostError;
use rust_decimal::Error as DecimalError;

#[derive(Debug, thiserror::Error)]
pub enum BillingContextError {
    #[error("the logging metadata {field} must be an object")]
    MetadataShape { field: &'static str },
    #[error("the logging model_info must be an object")]
    ModelInfoShape,
    #[error("the resolved pricing entry for {model} has no catalog key")]
    MissingKey { model: String },
    #[error("the stamped response time is not a decimal: {source}")]
    StampedTime {
        #[source]
        source: DecimalError,
    },
    #[error("the logging response time could not be computed: {source}")]
    Timing {
        #[from]
        source: TimestampError,
    },
    #[error("the OCR pricing could not be layered: {source}")]
    OcrPricing {
        #[from]
        source: OcrCostError,
    },
}
