use crate::finops::focus::destinations::focus_gcs_destination::GcsSettingsError;
use crate::finops::focus::destinations::focus_s3_destination::S3SettingsError;
use crate::finops::vantage::settings::SettingsError;
use crate::mavvrik_delivery::validation::ValidationError;

#[derive(Debug, thiserror::Error)]
pub enum FactoryError {
    #[error("Provider '{provider}' not supported for Focus export configuration")]
    UnsupportedProvider { provider: String },
    #[error("FOCUS_S3_BUCKET_NAME must be provided for S3 exports")]
    MissingS3BucketName,
    #[error("VANTAGE_API_KEY must be provided for Vantage exports")]
    MissingVantageApiKey,
    #[error("VANTAGE_INTEGRATION_TOKEN must be provided for Vantage exports")]
    MissingVantageIntegrationToken,
    #[error("FOCUS_GCS_BUCKET_NAME must be provided for GCS exports")]
    MissingGcsBucketName,
    #[error("the S3 FOCUS destination could not be built")]
    S3Destination(#[source] S3SettingsError),
    #[error("the Vantage FOCUS destination could not be built")]
    VantageDestination(#[source] SettingsError),
    #[error("the GCS FOCUS destination could not be built")]
    GcsDestination(#[source] GcsSettingsError),
    #[error("the Mavvrik FOCUS destination could not be built")]
    MavvrikDestination(#[source] ValidationError),
}
