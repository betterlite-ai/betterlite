use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use sha2::{Digest as _, Sha256};
use std::fmt::Debug;
use thiserror::Error;

use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;

use super::GcsBucketLogger;

const BUCKET_FIELD: &str = "gcs_bucket_name";
const ACCOUNT_FIELD: &str = "gcs_path_service_account";
const DEFAULT_KEY: &str = "default";

#[derive(Debug, Error)]
pub enum ConfigKeyError {
    #[error("standard_callback_dynamic_params must be an object")]
    Params,
    #[error("standard_callback_dynamic_params.{field} must be a string")]
    Field { field: &'static str },
}

pub fn config_key<Auth, Http>(
    logger: &GcsBucketLogger<Auth, Http>,
    dynamic: &Json,
) -> Result<String, ConfigKeyError>
where
    Auth: VertexAuth + Clone + Debug,
    Http: GcsHttpClient + Debug,
{
    let fields = match dynamic {
        Json::Null => None,
        Json::Object(fields) => Some(fields),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return Err(ConfigKeyError::Params);
        }
    };
    let bucket = text_field(fields, BUCKET_FIELD)?
        .or_else(|| logger.base.bucket_name.clone())
        .unwrap_or_else(|| DEFAULT_KEY.to_owned());
    let account = text_field(fields, ACCOUNT_FIELD)?
        .or_else(|| {
            logger
                .base
                .path_service_account
                .as_ref()
                .map(|secret| secret.expose_secret().to_owned())
        })
        .unwrap_or_else(|| DEFAULT_KEY.to_owned());
    Ok(format!("{bucket}|{account}"))
}

fn text_field(
    fields: Option<&IndexMap<String, Json>>,
    name: &'static str,
) -> Result<Option<String>, ConfigKeyError> {
    let Some(present) = fields else {
        return Ok(None);
    };
    match present.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ConfigKeyError::Field { field: name })
        }
    }
}

#[must_use]
pub fn sanitize_config_key(config_key: &str) -> String {
    let digest = hex::encode(Sha256::digest(config_key.as_bytes()));
    let short = digest.get(..8).unwrap_or_default();
    format!("config-{short}")
}

mod tests;
