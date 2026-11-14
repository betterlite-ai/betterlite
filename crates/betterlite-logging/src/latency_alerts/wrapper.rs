use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::hanging_requests::check::AlertingHangingRequestCheck;
use crate::hanging_requests::error::HangingRequestError;

pub async fn response_taking_too_long(
    check: &AlertingHangingRequestCheck,
    request_data: Option<&IndexMap<String, Json>>,
) -> Result<(), HangingRequestError> {
    let settings = check.settings.current();
    if !settings.alerting_configured {
        return Ok(());
    }
    if !settings
        .alert_types
        .contains(&AlertType::LlmRequestsHanging)
    {
        return Ok(());
    }
    check.add_hanging_request(request_data).await
}
