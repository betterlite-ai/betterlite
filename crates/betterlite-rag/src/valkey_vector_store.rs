pub mod error;
pub mod live;
pub mod params;
pub mod ports;
pub mod query;
pub mod reply;
mod tests;
mod wire;

use betterlite_openai::api::vector_store_records::records::vector::VectorStoreSearchResponse;
use betterlite_provider::api::vector_store_contracts::config::{Params, SearchQuery};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_encode};
use secrecy::{ExposeSecret as _, SecretString};

use error::ValkeyError;
use params::ValkeyParams;
use ports::{
    ValkeyConnector, ValkeyEmbedder, ValkeySearchParams, ValkeySearchPort, ValkeyTimeout,
    socket_timeouts,
};
use query::{knn_query, query_text};

// urllib appends these four to every safe set, and quote(password, safe='') keeps nothing else
const PYTHON_QUOTE_UNSAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

pub struct ValkeySearchRequest<'request> {
    pub vector_store_id: &'request str,
    pub query: &'request SearchQuery,
    pub optional_params: &'request Params,
    pub litellm_params: &'request Params,
    pub timeout: ValkeyTimeout,
}

#[must_use]
pub fn build_valkey_url(
    host: &str,
    port: u16,
    password: Option<&SecretString>,
    ssl: bool,
) -> String {
    let scheme = if ssl { "rediss" } else { "redis" };
    let credentials = match password {
        Some(secret) if !secret.expose_secret().is_empty() => {
            let encoded = percent_encode(secret.expose_secret().as_bytes(), PYTHON_QUOTE_UNSAFE);
            format!(":{encoded}@")
        }
        Some(_) | None => String::new(),
    };
    format!("{scheme}://{credentials}{host}:{port}")
}

#[must_use]
pub fn pack_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len().saturating_mul(4));
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

pub async fn search(
    request: &ValkeySearchRequest<'_>,
    embedder: &dyn ValkeyEmbedder,
    connector: &dyn ValkeyConnector,
    ready_port: Option<&dyn ValkeySearchPort>,
) -> Result<VectorStoreSearchResponse, ValkeyError> {
    let params = ValkeyParams::read(request.litellm_params)?;
    let text = query_text(request.query)?;
    let knn = knn_query(
        request.optional_params,
        &params.embedding_field,
        &params.text_field,
    )?;
    let model = params
        .embedding_model
        .as_deref()
        .ok_or(ValkeyError::EmbeddingModelRequired)?;
    let embeddings = embedder
        .embed(model, &text, &params.embedding_config)
        .await
        .map_err(|source| ValkeyError::Embedding { source })?;
    let vector = embeddings.first().ok_or(ValkeyError::NoEmbedding)?;
    let mut search_params = ValkeySearchParams::new();
    search_params.insert("vec".to_owned(), pack_vector(vector));
    let documents = if let Some(port) = ready_port {
        port.search(request.vector_store_id, &knn, &search_params)
            .await?
    } else {
        let url = params.connection_url()?;
        let port = connector
            .connect(&url, socket_timeouts(request.timeout))
            .await?;
        port.search(request.vector_store_id, &knn, &search_params)
            .await?
    };
    reply::search_response(&documents, &text, &params.text_field)
}
