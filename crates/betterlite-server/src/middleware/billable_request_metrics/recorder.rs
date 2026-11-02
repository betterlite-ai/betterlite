use axum::http::StatusCode;
use betterlite_billing::billable_category::BillableCategory;
use tower::BoxError;

// Called only for 2xx responses and exports them for enterprise metering.
pub trait BillingRecorder: Send + Sync {
    fn record(
        &self,
        category: BillableCategory,
        route: &str,
        status_code: StatusCode,
        model_id: Option<&str>,
    ) -> Result<(), BoxError>;
}
