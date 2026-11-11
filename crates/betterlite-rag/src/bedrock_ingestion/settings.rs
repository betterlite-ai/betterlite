use std::collections::BTreeMap;

use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::aws_endpoints::region::non_llm_region_name;
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};
use secrecy::SecretString;
use serde_json::Number;
use serde_json::value::RawValue;

pub const DEFAULT_EMBEDDING_MODEL: &str = "amazon.titan-embed-text-v2:0";
pub const DEFAULT_S3_PREFIX: &str = "data/";
pub const DEFAULT_INGESTION_TIMEOUT: i64 = 300;

pub type VectorStoreConfig = BTreeMap<String, Box<RawValue>>;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("{field} must be a string")]
    FieldType { field: &'static str },
    #[error("ingestion_timeout must be a whole number of seconds")]
    Timeout,
    #[error(transparent)]
    Region(#[from] AwsEndpointError),
}

// LiteLLM's BaseAWSLLM.get_credentials keys, cast with str() when the config holds one
#[derive(Debug, Clone)]
pub struct RagCredentials {
    pub access_key_id: Option<SecretString>,
    pub secret_access_key: Option<SecretString>,
    pub session_token: Option<SecretString>,
    pub session_name: Option<String>,
    pub profile_name: Option<String>,
    pub role_name: Option<String>,
    pub web_identity_token: Option<SecretString>,
    pub sts_endpoint: Option<String>,
    pub external_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BedrockRagSettings {
    pub knowledge_base_id: Option<String>,
    pub data_source_id: Option<String>,
    pub s3_bucket: Option<String>,
    // only the configured prefix; the working prefix lives on BedrockRagState
    pub s3_prefix: Option<String>,
    pub embedding_model: String,
    pub wait_for_ingestion: bool,
    pub ingestion_timeout: i64,
    pub aws_region_name: String,
    pub credentials: RagCredentials,
}

impl BedrockRagSettings {
    pub fn from_vector_store_config(
        config: &VectorStoreConfig,
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, SettingsError> {
        let vector_store_id = text(config, "vector_store_id")?.filter(|id| !id.is_empty());
        let knowledge_base_id = text(config, "knowledge_base_id")?.filter(|id| !id.is_empty());
        let data_source_id = text(config, "data_source_id")?.filter(|id| !id.is_empty());
        let s3_bucket = text(config, "s3_bucket")?.filter(|bucket| !bucket.is_empty());
        let s3_prefix = text(config, "s3_prefix")?.filter(|prefix| !prefix.is_empty());
        let embedding_model = text(config, "embedding_model")?
            .filter(|model| !model.is_empty())
            .unwrap_or_else(|| DEFAULT_EMBEDDING_MODEL.to_owned());
        let wait_for_ingestion = flag(config, "wait_for_ingestion")?.unwrap_or(false);
        let ingestion_timeout = timeout(config)?;
        let explicit_region = text(config, "aws_region_name")?.filter(|region| !region.is_empty());
        let aws_region_name = non_llm_region_name(
            explicit_region.as_deref(),
            environment.get("AWS_REGION_NAME").map(String::as_str),
            environment.get("AWS_REGION").map(String::as_str),
        )?;
        let credentials = RagCredentials {
            access_key_id: text(config, "aws_access_key_id")?.map(SecretString::from),
            secret_access_key: text(config, "aws_secret_access_key")?.map(SecretString::from),
            session_token: text(config, "aws_session_token")?.map(SecretString::from),
            session_name: text(config, "aws_session_name")?,
            profile_name: text(config, "aws_profile_name")?,
            role_name: text(config, "aws_role_name")?,
            web_identity_token: text(config, "aws_web_identity_token")?.map(SecretString::from),
            sts_endpoint: text(config, "aws_sts_endpoint")?,
            external_id: text(config, "aws_external_id")?,
        };
        Ok(Self {
            knowledge_base_id: vector_store_id.or(knowledge_base_id),
            data_source_id,
            s3_bucket,
            s3_prefix,
            embedding_model,
            wait_for_ingestion,
            ingestion_timeout,
            aws_region_name,
            credentials,
        })
    }
}

fn text(config: &VectorStoreConfig, field: &'static str) -> Result<Option<String>, SettingsError> {
    let Some(raw) = config.get(field) else {
        return Ok(None);
    };
    serde_json::from_str(raw.get()).map_err(|_source| SettingsError::FieldType { field })
}

fn flag(config: &VectorStoreConfig, field: &'static str) -> Result<Option<bool>, SettingsError> {
    let Some(raw) = config.get(field) else {
        return Ok(None);
    };
    serde_json::from_str(raw.get()).map_err(|_source| SettingsError::FieldType { field })
}

fn timeout(config: &VectorStoreConfig) -> Result<i64, SettingsError> {
    let Some(raw) = config.get("ingestion_timeout") else {
        return Ok(DEFAULT_INGESTION_TIMEOUT);
    };
    if raw.get().starts_with('"') {
        let text: String =
            serde_json::from_str(raw.get()).map_err(|_source| SettingsError::Timeout)?;
        return text
            .trim()
            .parse::<i64>()
            .map_err(|_source| SettingsError::Timeout);
    }
    let number: Option<Number> =
        serde_json::from_str(raw.get()).map_err(|_source| SettingsError::Timeout)?;
    number.map_or(Ok(DEFAULT_INGESTION_TIMEOUT), |seconds| {
        whole_seconds(&seconds)
    })
}

fn whole_seconds(number: &Number) -> Result<i64, SettingsError> {
    if let Some(seconds) = number.as_i64() {
        return Ok(seconds);
    }
    number
        .as_f64()
        .and_then(Decimal::from_f64)
        .and_then(|value| value.to_i64())
        .ok_or(SettingsError::Timeout)
}

mod tests;
