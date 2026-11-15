use betterlite_models::decisions::Json;
use chrono::{DateTime, Days, Utc};
use thiserror::Error;

use crate::gcs_client::gcs_storage_auth::download_behaviors::download_gcs_object;
use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;
use crate::types::gcs_storage_auth::type_definitions::GcsLoggingKwargs;

use super::GcsBucketLogger;
use super::naming_format_behaviors::{generate_success_object_name, get_object_date};

#[derive(Debug, Error)]
pub enum RetrievalError {
    #[error("start_time_utc is required for getting a payload from GCS Bucket")]
    MissingStartTime,
}

// only individually uploaded success objects carry a readable payload, so only their name is looked up
pub async fn get_request_response_payload<Auth, Http>(
    logger: &GcsBucketLogger<Auth, Http>,
    request_id: &str,
    start_time_utc: Option<DateTime<Utc>>,
) -> Result<Option<Json>, RetrievalError>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let Some(start) = start_time_utc else {
        return Err(RetrievalError::MissingStartTime);
    };
    for date in retrieval_dates(start) {
        let date_str = get_object_date(date);
        let object_name = generate_success_object_name(&date_str, request_id);
        let Some(body) =
            download_gcs_object(&logger.base, &object_name, &GcsLoggingKwargs::default()).await
        else {
            continue;
        };
        match serde_json::from_slice::<Json>(&body) {
            Ok(parsed) => return Ok(Some(parsed)),
            Err(error) => tracing::debug!(
                %error,
                date = %date_str,
                "GCS Bucket payload retrieval: the object body is not JSON"
            ),
        }
    }
    Ok(None)
}

fn retrieval_dates(start: DateTime<Utc>) -> [DateTime<Utc>; 3] {
    let next = start.checked_add_days(Days::new(1)).unwrap_or(start);
    let previous = start.checked_sub_days(Days::new(1)).unwrap_or(start);
    [start, next, previous]
}

mod tests;
