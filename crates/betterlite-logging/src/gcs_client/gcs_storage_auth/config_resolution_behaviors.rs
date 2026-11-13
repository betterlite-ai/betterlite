use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::SecretString;
use thiserror::Error;

use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;
use crate::types::gcs_storage_auth::type_definitions::{GcsLoggingConfig, GcsLoggingKwargs};

use super::GcsBucketBase;
use super::error::GcsClientError;
use super::vertex_caching_behaviors::get_or_create_vertex_instance;

const BUCKET_FIELD: &str = "gcs_bucket_name";
const SERVICE_ACCOUNT_FIELD: &str = "gcs_path_service_account";

#[derive(Debug, Error)]
pub enum GcsConfigError {
    #[error("standard_callback_dynamic_params must be an object")]
    Params,
    #[error("standard_callback_dynamic_params.{field} must be a string")]
    Field { field: &'static str },
}

pub async fn logging_config<Auth, Http>(
    base: &GcsBucketBase<Auth, Http>,
    kwargs: &GcsLoggingKwargs,
) -> Result<GcsLoggingConfig<Auth>, GcsClientError<Auth, Http>>
where
    Auth: VertexAuth + Clone,
    Http: GcsHttpClient,
{
    let dynamic = dynamic_fields(kwargs.dynamic_params.as_ref())?;
    let bucket_name = match dynamic {
        Some(fields) => text_field(fields, BUCKET_FIELD)?.or_else(|| base.bucket_name.clone()),
        None => base.bucket_name.clone(),
    };
    let path_service_account = match dynamic {
        Some(fields) => text_field(fields, SERVICE_ACCOUNT_FIELD)?
            .map(SecretString::from)
            .or_else(|| base.path_service_account.clone()),
        None => base.path_service_account.clone(),
    };
    let Some(resolved_bucket) = bucket_name else {
        return Err(GcsClientError::MissingBucketName);
    };
    let vertex_instance =
        get_or_create_vertex_instance(base, path_service_account.as_ref()).await?;
    Ok(GcsLoggingConfig {
        bucket_name: resolved_bucket,
        vertex_instance,
        path_service_account,
    })
}

const fn dynamic_fields(
    params: Option<&Json>,
) -> Result<Option<&IndexMap<String, Json>>, GcsConfigError> {
    match params {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(GcsConfigError::Params)
        }
    }
}

fn text_field(
    fields: &IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<String>, GcsConfigError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(GcsConfigError::Field { field: name })
        }
    }
}

mod tests;
