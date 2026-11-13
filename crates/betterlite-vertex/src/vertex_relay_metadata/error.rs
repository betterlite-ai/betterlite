use betterlite_models::cost_usage::error::CostUsageError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum VertexRelayError {
    #[error("the passthrough response body is not a JSON object")]
    ResponseBody {
        #[source]
        source: Box<serde_json::Error>,
    },
    #[error("the Interactions usage object could not be projected")]
    Usage {
        #[source]
        source: Box<CostUsageError>,
    },
}
