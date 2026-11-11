use crate::bedrock_ingestion::client::{
    AwsRagResponse, AwsRagTransport, BedrockRagClients, SignedRequest,
};
use crate::bedrock_ingestion::error::{BedrockIngestionError, aws_error_body};
use betterlite_config::rag::document::ingest::{
    RAGIngestEmbeddingOptions, S3VectorsVectorStoreOptions,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::error::Error as StdError;

mod tests;

pub const S3_VECTORS_SERVICE: &str = "s3vectors";
pub const S3_VECTORS_STORE_ID_ERROR: &str = "vector_store_id must be in format 'bucket_name:index_name' for S3 Vectors, or vector_bucket_name must be provided in litellm_params";
pub const DEFAULT_NON_FILTERABLE_KEYS: &[&str] = &["source_text"];
const CONTENT_SHA256: &str = "x-amz-content-sha256";
const GET_VECTOR_BUCKET: &str = "GetVectorBucket";
const CREATE_VECTOR_BUCKET: &str = "CreateVectorBucket";
const GET_INDEX: &str = "GetIndex";
const CREATE_INDEX: &str = "CreateIndex";
const PUT_VECTORS: &str = "PutVectors";

#[derive(Debug, thiserror::Error)]
pub enum S3VectorsInputError {
    #[error("{}", S3_VECTORS_STORE_ID_ERROR)]
    StoreId,
    #[error("vector bucket name '{0}' must be at least 3 characters")]
    BucketNameTooShort(String),
    #[error("vector bucket name '{0}' may only contain letters, numbers, hyphens and dots")]
    BucketNameCharacters(String),
}

#[derive(Debug, thiserror::Error)]
pub enum S3VectorsError<TransportFailure>
where
    TransportFailure: StdError,
{
    #[error(transparent)]
    Input(#[from] S3VectorsInputError),
    #[error("the S3 Vectors {0} request failed with status {1}: {2}")]
    Aws(&'static str, u16, String),
    #[error("the S3 Vectors {0} request body could not be serialized")]
    Serialize(&'static str, #[source] serde_json::Error),
    #[error(transparent)]
    Request(#[from] BedrockIngestionError<TransportFailure>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct S3VectorData {
    pub float32: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct S3VectorEntry {
    pub key: String,
    pub data: S3VectorData,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
pub struct VectorIndexConfig<'name> {
    pub bucket_name: &'name str,
    pub index_name: &'name str,
    pub dimension: i64,
    pub distance_metric: &'name str,
    pub non_filterable_metadata_keys: &'name [String],
}

pub struct S3VectorsRequests<'clients, 'deps, Transport> {
    pub clients: &'clients BedrockRagClients<'deps, Transport>,
    pub aws_region_name: &'clients str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BucketNameBody<'name> {
    vector_bucket_name: &'name str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IndexNameBody<'name> {
    vector_bucket_name: &'name str,
    index_name: &'name str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateIndexBody<'name> {
    vector_bucket_name: &'name str,
    index_name: &'name str,
    data_type: &'static str,
    dimension: i64,
    distance_metric: &'name str,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata_configuration: Option<MetadataConfiguration<'name>>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetadataConfiguration<'name> {
    non_filterable_metadata_keys: &'name [String],
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PutVectorsBody<'name> {
    vector_bucket_name: &'name str,
    index_name: &'name str,
    vectors: &'name [S3VectorEntry],
}

impl<Transport: AwsRagTransport> S3VectorsRequests<'_, '_, Transport> {
    async fn execute(
        &self,
        operation: &'static str,
        body: &[u8],
    ) -> Result<AwsRagResponse, S3VectorsError<Transport::Error>> {
        let url = format!(
            "https://s3vectors.{}.api.aws/{operation}",
            self.aws_region_name
        );
        let mut headers = Vec::with_capacity(2);
        if !body.is_empty() {
            headers.push(("Content-Type".to_owned(), "application/json".to_owned()));
        }
        headers.push((CONTENT_SHA256.to_owned(), hex::encode(Sha256::digest(body))));
        let request = SignedRequest {
            service: S3_VECTORS_SERVICE,
            signing_region: self.aws_region_name,
            method: "POST",
            url: &url,
            headers: &headers,
            body,
        };
        self.clients
            .signed_request_with(request)
            .await
            .map_err(S3VectorsError::Request)
    }

    pub async fn ensure_vector_bucket_exists(
        &self,
        bucket_name: &str,
    ) -> Result<(), S3VectorsError<Transport::Error>> {
        validate_bucket_name(bucket_name)?;
        let body = serde_json::to_vec(&BucketNameBody {
            vector_bucket_name: bucket_name,
        })
        .map_err(|source| S3VectorsError::Serialize(GET_VECTOR_BUCKET, source))?;
        let checked = self.execute(GET_VECTOR_BUCKET, &body).await;
        let exists = matches!(checked, Ok(response) if response.status == 200);
        if exists {
            return Ok(());
        }
        let response = self.execute(CREATE_VECTOR_BUCKET, &body).await?;
        require_status(CREATE_VECTOR_BUCKET, &response, &[200, 201, 409])
    }

    pub async fn ensure_vector_index_exists(
        &self,
        index: &VectorIndexConfig<'_>,
    ) -> Result<(), S3VectorsError<Transport::Error>> {
        let get_body = serde_json::to_vec(&IndexNameBody {
            vector_bucket_name: index.bucket_name,
            index_name: index.index_name,
        })
        .map_err(|source| S3VectorsError::Serialize(GET_INDEX, source))?;
        let checked = self.execute(GET_INDEX, &get_body).await;
        let exists = matches!(checked, Ok(response) if response.status == 200);
        if exists {
            return Ok(());
        }
        let metadata_configuration =
            (!index.non_filterable_metadata_keys.is_empty()).then_some(MetadataConfiguration {
                non_filterable_metadata_keys: index.non_filterable_metadata_keys,
            });
        let create_body = serde_json::to_vec(&CreateIndexBody {
            vector_bucket_name: index.bucket_name,
            index_name: index.index_name,
            data_type: "float32",
            dimension: index.dimension,
            distance_metric: index.distance_metric,
            metadata_configuration,
        })
        .map_err(|source| S3VectorsError::Serialize(CREATE_INDEX, source))?;
        let response = self.execute(CREATE_INDEX, &create_body).await?;
        require_status(CREATE_INDEX, &response, &[200, 201, 409])
    }

    pub async fn put_vectors(
        &self,
        vector_bucket_name: &str,
        index_name: &str,
        vectors: &[S3VectorEntry],
    ) -> Result<(), S3VectorsError<Transport::Error>> {
        let body = serde_json::to_vec(&PutVectorsBody {
            vector_bucket_name,
            index_name,
            vectors,
        })
        .map_err(|source| S3VectorsError::Serialize(PUT_VECTORS, source))?;
        let response = self.execute(PUT_VECTORS, &body).await?;
        require_status(PUT_VECTORS, &response, &[200, 201])
    }
}

fn require_status<TransportFailure>(
    operation: &'static str,
    response: &AwsRagResponse,
    accepted: &[u16],
) -> Result<(), S3VectorsError<TransportFailure>>
where
    TransportFailure: StdError,
{
    if accepted.contains(&response.status) {
        return Ok(());
    }
    let body = aws_error_body(&response.body);
    Err(S3VectorsError::Aws(operation, response.status, body))
}

fn validate_bucket_name(bucket_name: &str) -> Result<(), S3VectorsInputError> {
    if bucket_name.chars().count() < 3 {
        return Err(S3VectorsInputError::BucketNameTooShort(
            bucket_name.to_owned(),
        ));
    }
    let cleaned = bucket_name.replace(['-', '.'], "");
    if cleaned.is_empty() || !cleaned.chars().all(char::is_alphanumeric) {
        return Err(S3VectorsInputError::BucketNameCharacters(
            bucket_name.to_owned(),
        ));
    }
    Ok(())
}

pub fn ingest_target(
    config: &S3VectorsVectorStoreOptions,
) -> Result<(String, Option<String>), S3VectorsInputError> {
    let explicit_bucket_name = config
        .vector_bucket_name
        .as_deref()
        .filter(|name| !name.is_empty());
    let explicit_index_name = config.index_name.as_deref().filter(|name| !name.is_empty());
    let Some(store_id) = config
        .vector_store_id
        .as_deref()
        .filter(|id| !id.is_empty())
    else {
        let Some(bucket_name) = explicit_bucket_name else {
            return Err(S3VectorsInputError::StoreId);
        };
        return Ok((
            bucket_name.to_owned(),
            explicit_index_name.map(str::to_owned),
        ));
    };
    let (derived_bucket_name, derived_index_name) = split_store_id(store_id, explicit_bucket_name)?;
    let bucket_name = explicit_bucket_name.unwrap_or(derived_bucket_name);
    let index_name = explicit_index_name.unwrap_or(derived_index_name);
    Ok((bucket_name.to_owned(), Some(index_name.to_owned())))
}

fn split_store_id<'id>(
    store_id: &'id str,
    fallback_bucket_name: Option<&'id str>,
) -> Result<(&'id str, &'id str), S3VectorsInputError> {
    let (bucket_name, index_name) = match store_id.split_once(':') {
        Some(parts) => parts,
        None => (
            fallback_bucket_name.ok_or(S3VectorsInputError::StoreId)?,
            store_id,
        ),
    };
    if bucket_name.is_empty() || index_name.is_empty() {
        return Err(S3VectorsInputError::StoreId);
    }
    Ok((bucket_name, index_name))
}

#[must_use]
pub fn ingest_embedding_options(
    config: &S3VectorsVectorStoreOptions,
    embedding_options: Option<RAGIngestEmbeddingOptions>,
) -> Option<RAGIngestEmbeddingOptions> {
    let store_model = config
        .litellm_embedding_model
        .as_deref()
        .filter(|model| !model.is_empty())
        .or_else(|| {
            config
                .embedding_model
                .as_deref()
                .filter(|model| !model.is_empty())
        });
    store_model
        .map(|model| RAGIngestEmbeddingOptions {
            model: Some(model.to_owned()),
        })
        .or(embedding_options)
}
