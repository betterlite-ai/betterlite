use std::io;

use axum::http::StatusCode;
use betterlite_billing::billable_category::BillableCategory;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BillingRecorderError {
    #[error("billing recorder failed: {0}")]
    Failed(#[source] io::Error),
}

// Called only for 2xx responses and exports them for enterprise metering.
pub trait BillingRecorder: Send + Sync {
    fn record(
        &self,
        category: BillableCategory,
        route: &str,
        status_code: StatusCode,
        model_id: Option<&str>,
    ) -> Result<(), BillingRecorderError>;
}
