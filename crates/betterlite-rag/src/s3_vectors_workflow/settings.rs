use std::collections::BTreeMap;

use betterlite_bedrock::aws_endpoints::error::AwsEndpointError;
use betterlite_bedrock::aws_endpoints::region::non_llm_region_name;
use betterlite_config::rag::document::ingest::{
    RAGIngestEmbeddingOptions, S3VectorsDistanceMetric, S3VectorsVectorStoreOptions, SslVerify,
};
use betterlite_env::{EnvError, number};
use secrecy::ExposeSecret as _;

use crate::bedrock_ingestion::settings::RagCredentials;
use crate::s3_vectors_ingestion::{
    DEFAULT_NON_FILTERABLE_KEYS, S3VectorsInputError, ingest_embedding_options, ingest_target,
};

pub const DEFAULT_DIMENSION: i64 = 1024;
pub const DEFAULT_DISTANCE_METRIC: &str = "cosine";
const DIMENSION_ENV: &str = "S3_VECTORS_DEFAULT_DIMENSION";
const DISTANCE_METRIC_ENV: &str = "S3_VECTORS_DEFAULT_DISTANCE_METRIC";

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error(transparent)]
    Target(#[from] S3VectorsInputError),
    #[error(transparent)]
    Region(#[from] AwsEndpointError),
    #[error(transparent)]
    Environment(#[from] EnvError),
}

#[derive(Debug, Clone)]
pub struct S3VectorsSettings {
    pub vector_bucket_name: String,
    pub index_name: Option<String>,
    pub embedding_model: Option<String>,
    pub distance_metric: String,
    pub non_filterable_metadata_keys: Vec<String>,
    pub dimension: Option<i64>,
    pub default_dimension: i64,
    pub aws_region_name: String,
    pub credentials: RagCredentials,
    pub ssl_verify: Option<SslVerify>,
}

impl S3VectorsSettings {
    pub fn from_vector_store_config(
        config: &S3VectorsVectorStoreOptions,
        embedding_options: Option<RAGIngestEmbeddingOptions>,
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, SettingsError> {
        let (vector_bucket_name, index_name) = ingest_target(config)?;
        Ok(Self {
            vector_bucket_name,
            index_name,
            embedding_model: ingest_embedding_options(config, embedding_options)
                .and_then(|options| options.model),
            distance_metric: distance_metric(config, environment),
            non_filterable_metadata_keys: non_filterable_keys(config),
            dimension: config.dimension,
            default_dimension: default_dimension(environment)?,
            aws_region_name: region_name(config, environment)?,
            credentials: credentials(config),
            ssl_verify: config.ssl_verify.clone(),
        })
    }
}

fn distance_metric(
    config: &S3VectorsVectorStoreOptions,
    environment: &BTreeMap<String, String>,
) -> String {
    let Some(configured) = config.distance_metric else {
        return environment
            .get(DISTANCE_METRIC_ENV)
            .filter(|value| !value.is_empty())
            .map_or_else(|| DEFAULT_DISTANCE_METRIC.to_owned(), Clone::clone);
    };
    match configured {
        S3VectorsDistanceMetric::Cosine => "cosine".to_owned(),
        S3VectorsDistanceMetric::Euclidean => "euclidean".to_owned(),
    }
}

fn non_filterable_keys(config: &S3VectorsVectorStoreOptions) -> Vec<String> {
    config
        .non_filterable_metadata_keys
        .clone()
        .unwrap_or_else(|| {
            DEFAULT_NON_FILTERABLE_KEYS
                .iter()
                .map(|key| (*key).to_owned())
                .collect()
        })
}

fn default_dimension(environment: &BTreeMap<String, String>) -> Result<i64, SettingsError> {
    let Some(text) = environment.get(DIMENSION_ENV) else {
        return Ok(DEFAULT_DIMENSION);
    };
    Ok(number::<i64>(DIMENSION_ENV, text)?.unwrap_or(DEFAULT_DIMENSION))
}

fn region_name(
    config: &S3VectorsVectorStoreOptions,
    environment: &BTreeMap<String, String>,
) -> Result<String, SettingsError> {
    non_llm_region_name(
        config
            .aws_region_name
            .as_deref()
            .filter(|region| !region.is_empty()),
        environment.get("AWS_REGION_NAME").map(String::as_str),
        environment.get("AWS_REGION").map(String::as_str),
    )
    .map_err(SettingsError::from)
}

fn credentials(config: &S3VectorsVectorStoreOptions) -> RagCredentials {
    RagCredentials {
        access_key_id: config.aws_access_key_id.clone(),
        secret_access_key: config.aws_secret_access_key.clone(),
        session_token: config.aws_session_token.clone(),
        session_name: config.aws_session_name.clone(),
        profile_name: config.aws_profile_name.clone(),
        role_name: config.aws_role_name.clone(),
        web_identity_token: config.aws_web_identity_token.clone(),
        sts_endpoint: config
            .aws_sts_endpoint
            .as_ref()
            .map(|value| value.expose_secret().to_owned()),
        external_id: config
            .aws_external_id
            .as_ref()
            .map(|value| value.expose_secret().to_owned()),
    }
}
