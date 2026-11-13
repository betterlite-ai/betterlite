use http::Method;

use super::QdrantSemanticCache;
use super::decode;
use super::error::QdrantSemanticError;
use super::options::{CACHE_KEY_FIELD_NAME, Quantization};
use super::ports::{QdrantReply, QdrantRequest};
use super::wire::{
    BinaryParams, CollectionExistsResponse, CreateCollectionRequest, CreateCollectionResponse,
    PayloadIndexRequest, ProductParams, QuantizationParams, ScalarParams, VectorParams,
};

pub(crate) async fn ensure_collection(
    cache: &QdrantSemanticCache,
) -> Result<(), QdrantSemanticError> {
    let path = format!("/collections/{}/exists", cache.collection_name);
    let reply = send(cache, Method::GET, path, None).await?;
    if reply.status != 200 {
        return Err(QdrantSemanticError::CollectionCheck {
            body: String::from_utf8(reply.body).unwrap_or_default(),
        });
    }
    let exists: CollectionExistsResponse = decode(&reply.body, "collection exists")?;
    if exists.result.exists {
        return Ok(());
    }
    let body = create_body(cache)?;
    let created = cache
        .http_client
        .send(QdrantRequest {
            method: Method::PUT,
            path: format!("/collections/{}", cache.collection_name),
            query: Vec::new(),
            body: Some(body),
        })
        .await?;
    confirm_created(&created)
}

fn create_body(cache: &QdrantSemanticCache) -> Result<Vec<u8>, QdrantSemanticError> {
    let request = CreateCollectionRequest {
        vectors: VectorParams {
            size: cache.vector_size,
            distance: "Cosine",
        },
        quantization_config: quantization_params(cache)?,
    };
    serde_json::to_vec(&request).map_err(QdrantSemanticError::Encode)
}

fn confirm_created(reply: &QdrantReply) -> Result<(), QdrantSemanticError> {
    let created: CreateCollectionResponse = decode(&reply.body, "collection creation")?;
    if created.result {
        return Ok(());
    }
    Err(QdrantSemanticError::CreateCollection)
}

fn quantization_params(
    cache: &QdrantSemanticCache,
) -> Result<QuantizationParams, QdrantSemanticError> {
    Ok(
        match Quantization::parse(cache.quantization_config.as_deref())? {
            Quantization::Binary => QuantizationParams::Binary(BinaryParams { always_ram: false }),
            Quantization::Scalar => QuantizationParams::Scalar(ScalarParams {
                r#type: "int8",
                quantile: cache.scalar_quantile,
                always_ram: false,
            }),
            Quantization::Product => QuantizationParams::Product(ProductParams {
                compression: "x16",
                always_ram: false,
            }),
        },
    )
}

// LiteLLM fetches the details into collection_info, whose only reader has no caller at the pin
pub(crate) async fn read_collection_details(
    cache: &QdrantSemanticCache,
) -> Result<(), QdrantSemanticError> {
    let path = format!("/collections/{}", cache.collection_name);
    let _details = send(cache, Method::GET, path, None).await?;
    Ok(())
}

pub(crate) async fn ensure_payload_index(cache: &QdrantSemanticCache) {
    let body = PayloadIndexRequest {
        field_name: CACHE_KEY_FIELD_NAME,
        field_schema: "keyword",
    };
    let Ok(encoded) = serde_json::to_vec(&body).map_err(QdrantSemanticError::Encode) else {
        tracing::warn!(
            "Qdrant semantic-cache could not create cache-key payload index: the request body could not be serialized"
        );
        return;
    };
    let path = format!("/collections/{}/index", cache.collection_name);
    match send(cache, Method::PUT, path, Some(encoded)).await {
        Ok(reply) if reply.status == 200 || reply.status == 201 => {}
        Ok(reply) => tracing::warn!(
            "Qdrant semantic-cache could not create cache-key payload index: {}",
            String::from_utf8(reply.body).unwrap_or_default()
        ),
        Err(error) => tracing::warn!(
            "Qdrant semantic-cache could not create cache-key payload index: {error}"
        ),
    }
}

async fn send(
    cache: &QdrantSemanticCache,
    method: Method,
    path: String,
    body: Option<Vec<u8>>,
) -> Result<QdrantReply, QdrantSemanticError> {
    let request = QdrantRequest {
        method,
        path,
        query: Vec::new(),
        body,
    };
    cache.http_client.send(request).await
}
