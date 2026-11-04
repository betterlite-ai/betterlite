use std::sync::Arc;

use google_cloud_kms_v1::client::KeyManagementService;

use super::{GOOGLE_KMS_CLIENT, GOOGLE_KMS_RESOURCE_NAME, GoogleKmsError};
use crate::aws_kms::environment_snapshot;

pub fn validate_environment() -> Result<(String, String), GoogleKmsError> {
    let environment = environment_snapshot();
    validate_values(
        environment
            .get("GOOGLE_APPLICATION_CREDENTIALS")
            .map(String::as_str),
        environment
            .get("GOOGLE_KMS_RESOURCE_NAME")
            .map(String::as_str),
    )
}

fn validate_values(
    credentials_path: Option<&str>,
    configured_resource_name: Option<&str>,
) -> Result<(String, String), GoogleKmsError> {
    let credentials = credentials_path.ok_or(GoogleKmsError::MissingCredentials)?;
    let resource_name = configured_resource_name.ok_or(GoogleKmsError::MissingResourceName)?;
    Ok((credentials.to_owned(), resource_name.to_owned()))
}

pub async fn load_google_kms(use_google_kms: Option<bool>) -> Result<(), GoogleKmsError> {
    if use_google_kms != Some(true) {
        return Ok(());
    }
    let (_credentials_path, resource_name) = validate_environment()?;
    let client = KeyManagementService::builder()
        .build()
        .await
        .map_err(|error| GoogleKmsError::Client(error.to_string()))?;
    let mut stored_client = GOOGLE_KMS_CLIENT
        .lock()
        .map_err(|_poisoned| GoogleKmsError::ClientState)?;
    let mut stored_resource_name = GOOGLE_KMS_RESOURCE_NAME
        .lock()
        .map_err(|_poisoned| GoogleKmsError::ClientState)?;
    if stored_client.is_some() || stored_resource_name.is_some() {
        return Err(GoogleKmsError::Client(
            "Google KMS client is already initialized".to_owned(),
        ));
    }
    *stored_client = Some(Arc::new(client));
    *stored_resource_name = Some(resource_name);
    drop(stored_resource_name);
    drop(stored_client);
    Ok(())
}

mod tests;
