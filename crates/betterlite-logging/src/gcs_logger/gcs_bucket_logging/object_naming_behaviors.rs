use betterlite_context::cloud_storage::sanitize_component;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use thiserror::Error;

use crate::fields::payload::StandardLoggingPayload;
use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;

use super::GcsBucketLogger;
use super::naming_format_behaviors::{
    generate_failure_object_name, generate_success_object_name, get_object_date,
};

const GCS_LOG_ID_FIELD: &str = "gcs_log_id";

#[derive(Debug, Error)]
pub enum ObjectNameError {
    #[error("litellm_params.metadata.gcs_log_id must be a string")]
    GcsLogId,
}

pub fn get_object_name<Auth, Http>(
    logger: &GcsBucketLogger<Auth, Http>,
    payload: &StandardLoggingPayload,
    response_id: &str,
    metadata: &IndexMap<String, Json>,
) -> Result<String, ObjectNameError>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let date = get_object_date(logger.clock.now());
    let mut object_name = if payload.error_str.is_none() {
        generate_success_object_name(&date, response_id)
    } else {
        generate_failure_object_name(&date, &logger.ids.uuid_hex())
    };
    let Some(value) = metadata.get(GCS_LOG_ID_FIELD) else {
        return Ok(object_name);
    };
    let safe_log_id = match value {
        Json::Text(text) => sanitize_component(Some(text), ""),
        Json::Null => String::new(),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            return Err(ObjectNameError::GcsLogId);
        }
    };
    if !safe_log_id.is_empty() {
        object_name = format!("{date}/custom-{}-{safe_log_id}", logger.ids.uuid_hex());
    }
    Ok(object_name)
}

mod tests;
