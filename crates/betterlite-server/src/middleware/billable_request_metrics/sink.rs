use std::io;

use axum::http::StatusCode;
use betterlite_billing::billable_category::BillableCategory;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GatewayRequestSinkError {
    #[error("gateway request sink failed: {0}")]
    Failed(#[source] io::Error),
}

// Unlike BillingRecorder, this runs for every status code, takes no model id, and omits the serving deployment on purpose.
pub trait GatewayRequestSink: Send + Sync {
    fn record(
        &self,
        category: BillableCategory,
        route: &str,
        status_code: StatusCode,
    ) -> Result<(), GatewayRequestSinkError>;
}
