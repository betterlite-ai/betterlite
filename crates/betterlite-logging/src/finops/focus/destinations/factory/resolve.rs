use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};

use crate::finops::focus::destinations::focus_gcs_destination::GcsSettings;
use crate::finops::focus::destinations::focus_s3_destination::S3Settings;
use crate::finops::vantage::settings::{DEFAULT_BASE_URL, VantageSettings};
use crate::mavvrik_delivery::destination::MavvrikSettings;

use super::error::FactoryError;

const S3_BUCKET_NAME_VARIABLE: &str = "FOCUS_S3_BUCKET_NAME";
const S3_REGION_VARIABLE: &str = "FOCUS_S3_REGION_NAME";
const S3_ENDPOINT_VARIABLE: &str = "FOCUS_S3_ENDPOINT_URL";
const S3_ACCESS_KEY_VARIABLE: &str = "FOCUS_S3_ACCESS_KEY";
const S3_SECRET_KEY_VARIABLE: &str = "FOCUS_S3_SECRET_KEY";
const S3_SESSION_TOKEN_VARIABLE: &str = "FOCUS_S3_SESSION_TOKEN";
const VANTAGE_API_KEY_VARIABLE: &str = "VANTAGE_API_KEY";
const VANTAGE_TOKEN_VARIABLE: &str = "VANTAGE_INTEGRATION_TOKEN";
const VANTAGE_BASE_URL_VARIABLE: &str = "VANTAGE_BASE_URL";
const GCS_BUCKET_NAME_VARIABLE: &str = "FOCUS_GCS_BUCKET_NAME";
const GCS_SERVICE_ACCOUNT_VARIABLE: &str = "FOCUS_GCS_PATH_SERVICE_ACCOUNT";
const MAVVRIK_API_KEY_VARIABLE: &str = "MAVVRIK_API_KEY";
const MAVVRIK_API_ENDPOINT_VARIABLE: &str = "MAVVRIK_API_ENDPOINT";
const MAVVRIK_CONNECTION_ID_VARIABLE: &str = "MAVVRIK_CONNECTION_ID";

#[derive(Clone, Debug, Default)]
pub struct DestinationOverrides {
    pub bucket_name: Option<String>,
    pub region_name: Option<String>,
    pub endpoint_url: Option<String>,
    pub aws_access_key_id: Option<SecretString>,
    pub aws_secret_access_key: Option<SecretString>,
    pub aws_session_token: Option<SecretString>,
    pub api_key: Option<SecretString>,
    pub integration_token: Option<SecretString>,
    pub base_url: Option<String>,
    pub service_account_json: Option<SecretString>,
    pub api_endpoint: Option<String>,
    pub connection_id: Option<String>,
}

fn text_or_environment(
    value: Option<&str>,
    variable: &str,
    environment: &dyn EnvironmentLookup,
) -> Option<String> {
    match value {
        Some(value) => Some(value.to_owned()),
        None => environment
            .lookup(variable)
            .filter(|found| !found.is_empty()),
    }
}

fn secret_or_environment(
    value: Option<&SecretString>,
    variable: &str,
    environment: &dyn EnvironmentLookup,
) -> Option<SecretString> {
    match value {
        Some(value) => Some(value.clone()),
        None => environment
            .lookup(variable)
            .filter(|found| !found.is_empty())
            .map(SecretString::from),
    }
}

fn require_secret(
    value: Option<SecretString>,
    missing: FactoryError,
) -> Result<SecretString, FactoryError> {
    match value {
        Some(value) if !value.expose_secret().is_empty() => Ok(value),
        _ => Err(missing),
    }
}

pub(super) fn s3(
    overrides: &DestinationOverrides,
    environment: &dyn EnvironmentLookup,
) -> Result<S3Settings, FactoryError> {
    let settings = S3Settings {
        bucket_name: text_or_environment(
            overrides.bucket_name.as_deref(),
            S3_BUCKET_NAME_VARIABLE,
            environment,
        ),
        region_name: text_or_environment(
            overrides.region_name.as_deref(),
            S3_REGION_VARIABLE,
            environment,
        ),
        endpoint_url: text_or_environment(
            overrides.endpoint_url.as_deref(),
            S3_ENDPOINT_VARIABLE,
            environment,
        ),
        aws_access_key_id: secret_or_environment(
            overrides.aws_access_key_id.as_ref(),
            S3_ACCESS_KEY_VARIABLE,
            environment,
        ),
        aws_secret_access_key: secret_or_environment(
            overrides.aws_secret_access_key.as_ref(),
            S3_SECRET_KEY_VARIABLE,
            environment,
        ),
        aws_session_token: secret_or_environment(
            overrides.aws_session_token.as_ref(),
            S3_SESSION_TOKEN_VARIABLE,
            environment,
        ),
    };
    if settings.bucket_name.as_deref().is_none_or(str::is_empty) {
        return Err(FactoryError::MissingS3BucketName);
    }
    Ok(settings)
}

pub(super) fn vantage(
    overrides: &DestinationOverrides,
    environment: &dyn EnvironmentLookup,
) -> Result<VantageSettings, FactoryError> {
    let configured_api_key = secret_or_environment(
        overrides.api_key.as_ref(),
        VANTAGE_API_KEY_VARIABLE,
        environment,
    );
    let configured_integration_token = secret_or_environment(
        overrides.integration_token.as_ref(),
        VANTAGE_TOKEN_VARIABLE,
        environment,
    );
    let api_key = require_secret(configured_api_key, FactoryError::MissingVantageApiKey)?;
    let integration_token = require_secret(
        configured_integration_token,
        FactoryError::MissingVantageIntegrationToken,
    )?;
    let base_url = text_or_environment(
        overrides.base_url.as_deref(),
        VANTAGE_BASE_URL_VARIABLE,
        environment,
    )
    .or_else(|| Some(DEFAULT_BASE_URL.to_owned()));
    Ok(VantageSettings {
        api_key: Some(api_key),
        integration_token: Some(integration_token),
        base_url,
    })
}

pub(super) fn gcs(
    overrides: &DestinationOverrides,
    environment: &dyn EnvironmentLookup,
) -> Result<GcsSettings, FactoryError> {
    let bucket_name = text_or_environment(
        overrides.bucket_name.as_deref(),
        GCS_BUCKET_NAME_VARIABLE,
        environment,
    );
    let service_account_json = secret_or_environment(
        overrides.service_account_json.as_ref(),
        GCS_SERVICE_ACCOUNT_VARIABLE,
        environment,
    );
    if bucket_name.as_deref().is_none_or(str::is_empty) {
        return Err(FactoryError::MissingGcsBucketName);
    }
    Ok(GcsSettings {
        bucket_name,
        service_account_json,
    })
}

pub(super) fn mavvrik(
    overrides: &DestinationOverrides,
    environment: &dyn EnvironmentLookup,
) -> MavvrikSettings {
    MavvrikSettings {
        api_key: secret_or_environment(
            overrides.api_key.as_ref(),
            MAVVRIK_API_KEY_VARIABLE,
            environment,
        ),
        api_endpoint: text_or_environment(
            overrides.api_endpoint.as_deref(),
            MAVVRIK_API_ENDPOINT_VARIABLE,
            environment,
        ),
        connection_id: text_or_environment(
            overrides.connection_id.as_deref(),
            MAVVRIK_CONNECTION_ID_VARIABLE,
            environment,
        ),
    }
}
