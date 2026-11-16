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

enum Text {
    S3Bucket,
    GcsBucket,
    S3Region,
    S3Endpoint,
    VantageBaseUrl,
    MavvrikApiEndpoint,
    MavvrikConnectionId,
}

enum Credential {
    S3AccessKey,
    S3SecretKey,
    S3SessionToken,
    GcsServiceAccount,
    VantageApiKey,
    VantageToken,
    MavvrikApiKey,
}

// a setting comes from the override first and from its environment variable only when the override is absent
pub(super) struct Sources<'a> {
    pub(super) overrides: &'a DestinationOverrides,
    pub(super) environment: &'a dyn EnvironmentLookup,
}

impl Sources<'_> {
    fn text(&self, setting: Text) -> Option<String> {
        let (chosen, variable) = match setting {
            Text::S3Bucket => (
                self.overrides.bucket_name.as_deref(),
                S3_BUCKET_NAME_VARIABLE,
            ),
            Text::GcsBucket => (
                self.overrides.bucket_name.as_deref(),
                GCS_BUCKET_NAME_VARIABLE,
            ),
            Text::S3Region => (self.overrides.region_name.as_deref(), S3_REGION_VARIABLE),
            Text::S3Endpoint => (self.overrides.endpoint_url.as_deref(), S3_ENDPOINT_VARIABLE),
            Text::VantageBaseUrl => (
                self.overrides.base_url.as_deref(),
                VANTAGE_BASE_URL_VARIABLE,
            ),
            Text::MavvrikApiEndpoint => (
                self.overrides.api_endpoint.as_deref(),
                MAVVRIK_API_ENDPOINT_VARIABLE,
            ),
            Text::MavvrikConnectionId => (
                self.overrides.connection_id.as_deref(),
                MAVVRIK_CONNECTION_ID_VARIABLE,
            ),
        };
        chosen.map(str::to_owned).or_else(|| {
            self.environment
                .lookup(variable)
                .filter(|found| !found.is_empty())
        })
    }

    fn secret(&self, credential: Credential) -> Option<SecretString> {
        let (chosen, variable) = match credential {
            Credential::S3AccessKey => (
                self.overrides.aws_access_key_id.as_ref(),
                S3_ACCESS_KEY_VARIABLE,
            ),
            Credential::S3SecretKey => (
                self.overrides.aws_secret_access_key.as_ref(),
                S3_SECRET_KEY_VARIABLE,
            ),
            Credential::S3SessionToken => (
                self.overrides.aws_session_token.as_ref(),
                S3_SESSION_TOKEN_VARIABLE,
            ),
            Credential::GcsServiceAccount => (
                self.overrides.service_account_json.as_ref(),
                GCS_SERVICE_ACCOUNT_VARIABLE,
            ),
            Credential::VantageApiKey => {
                (self.overrides.api_key.as_ref(), VANTAGE_API_KEY_VARIABLE)
            }
            Credential::VantageToken => (
                self.overrides.integration_token.as_ref(),
                VANTAGE_TOKEN_VARIABLE,
            ),
            Credential::MavvrikApiKey => {
                (self.overrides.api_key.as_ref(), MAVVRIK_API_KEY_VARIABLE)
            }
        };
        chosen
            .map(SecretString::expose_secret)
            .map(str::to_owned)
            .or_else(|| {
                self.environment
                    .lookup(variable)
                    .filter(|found| !found.is_empty())
            })
            .map(SecretString::from)
    }

    fn s3_bucket(&self) -> Result<String, FactoryError> {
        match self.text(Text::S3Bucket) {
            Some(name) if !name.is_empty() => Ok(name),
            _ => Err(FactoryError::MissingS3BucketName),
        }
    }

    fn gcs_bucket(&self) -> Result<String, FactoryError> {
        match self.text(Text::GcsBucket) {
            Some(name) if !name.is_empty() => Ok(name),
            _ => Err(FactoryError::MissingGcsBucketName),
        }
    }

    fn vantage_api_key(&self) -> Result<SecretString, FactoryError> {
        match self.secret(Credential::VantageApiKey) {
            Some(secret) if !secret.expose_secret().is_empty() => Ok(secret),
            _ => Err(FactoryError::MissingVantageApiKey),
        }
    }

    fn vantage_token(&self) -> Result<SecretString, FactoryError> {
        match self.secret(Credential::VantageToken) {
            Some(secret) if !secret.expose_secret().is_empty() => Ok(secret),
            _ => Err(FactoryError::MissingVantageIntegrationToken),
        }
    }

    fn vantage_base_url(&self) -> Option<String> {
        self.text(Text::VantageBaseUrl)
            .or_else(|| Some(DEFAULT_BASE_URL.to_owned()))
    }
}

pub(super) fn s3(sources: &Sources<'_>) -> Result<S3Settings, FactoryError> {
    Ok(S3Settings {
        bucket_name: Some(sources.s3_bucket()?),
        region_name: sources.text(Text::S3Region),
        endpoint_url: sources.text(Text::S3Endpoint),
        aws_access_key_id: sources.secret(Credential::S3AccessKey),
        aws_secret_access_key: sources.secret(Credential::S3SecretKey),
        aws_session_token: sources.secret(Credential::S3SessionToken),
    })
}

pub(super) fn vantage(sources: &Sources<'_>) -> Result<VantageSettings, FactoryError> {
    Ok(VantageSettings {
        api_key: Some(sources.vantage_api_key()?),
        integration_token: Some(sources.vantage_token()?),
        base_url: sources.vantage_base_url(),
    })
}

pub(super) fn gcs(sources: &Sources<'_>) -> Result<GcsSettings, FactoryError> {
    Ok(GcsSettings {
        bucket_name: Some(sources.gcs_bucket()?),
        service_account_json: sources.secret(Credential::GcsServiceAccount),
    })
}

pub(super) fn mavvrik(sources: &Sources<'_>) -> MavvrikSettings {
    MavvrikSettings {
        api_key: sources.secret(Credential::MavvrikApiKey),
        api_endpoint: sources.text(Text::MavvrikApiEndpoint),
        connection_id: sources.text(Text::MavvrikConnectionId),
    }
}
